<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# One resolution context, and making name visibility observable

> **Status: BUILT (2026-07-27).** All four steps shipped; § What the build corrected
> records the three places the design was wrong, two of them found by the probes it
> named. Gates: `tests/repl_session.rs::repl_keeps_its_resolution_context_across_a_reset`
> (Part A, both silent sites), `cache::tests::{cache_decision_precedence,
> dev_build_probe_reads_the_binary_path}` (B3), and the B2 guard, whose non-vacuity was
> proven by disabling `replay_imports` — it then fires *at the rollback* and names the
> alias.
>
> ORIGINAL DESIGN BELOW. Two shipped fixes — @PLN120 **E.1**
> and **E.4** — share a root this design names, and both were found by a *consumer*
> rather than by us. The fix is small. The larger half is **Part B**: the tooling that
> would have shown either one, because in both cases the state that was wrong could
> not be looked at, only guessed at and then hand-instrumented.

## The two defects, and what they have in common

| | what shipped | what was wrong |
|---|---|---|
| **E.1** | `run_file_debug` now takes `lib_dirs` | `--lib` was wired into 3 of 4 entry points; the 4th built a stdlib-only session, so `loft debug` reported `Library 'x' not found` on a file that runs fine |
| **E.4** | `Data` retains applied imports; `rebuild_indices` replays them | a rebuild reconstructs `def_names` from `definitions`, which know only their own source, so every *import alias* was dropped — and the REPL rebuilds on every eval probe |

Different code, one root: **the set of names a source can see is assembled from loose
parts and is nowhere observable.** E.1 lost a part on the way in (a parameter not
passed); E.4 lost it in flight (derived state not reproduced). Neither could be *seen*
— each presented as "this name does not exist", which reads as a missing feature.

Both were reported by moros. That is the tell worth acting on: a consumer noticing
your resolution state is broken before you do means you have no view of it.

## Part A — one resolution context

### The sites, counted before any code (design-protocol step 2)

A session's resolution inputs are passed as loose parameters (`stdlib_dir`, then
`lib_dirs` if the caller remembers). Six sites build one:

| # | site | context | passes `lib_dirs`? |
|---|---|---|---|
| 1 | `repl.rs:151` `run_repl` | `loft repl` | ❌ **no** — `loft repl --lib d` cannot see the library |
| 2 | `repl.rs:193` `run_file_debug` | `loft debug f:N` | ✓ (E.1's fix) |
| 3 | `rpc.rs:85` `run_rpc` | `loft debug --rpc` | ✓ |
| 4 | `serve.rs:46` `run_serve` | `loft --serve` | ✓ |
| 5 | `live_reload.rs:90` | live reload | ✓ |
| 6 | `repl.rs:680` `:reset` | inside a live session | ❌ **drops them** |

**Two of six are wrong and both are silent.** Site 1 is E.1's defect at a fourth
entry point — verified: `loft repl --lib <dir>` answers *"Library 'geom' not found"*.
Site 6 is worse than a missing flag: it silently **un-libs a session that was
working**, because `run_loop` is threaded `stdlib_dir` alone and so *structurally
cannot* restore `lib_dirs`. `N × silence` is 2 × silent, and the shape guarantees a
seventh site will be added wrong.

**Site 6 is latent behind site 1, and that is the interesting part.** It cannot be
demonstrated at runtime today: `:reset` is a top-level REPL command (not available at
the `(dbg)` prompt), and the only way to get a library into a plain REPL session is
`loft repl --lib` — which is site 1. So the proof is by construction (no `lib_dirs`
are in scope where `:reset` rebuilds the session), and **fixing site 1 alone would
expose site 6** rather than complete the job. Two silent faults where the first hides
the second is exactly the shape that argues for the one-value fix over two local
patches.

### The invariant

> *A session's resolution inputs travel as one value. There is no way to construct a
> session with some of them.*

```rust
/// Everything that decides which names a source can see.  One value, so a session
/// cannot be built with a subset — the shape that put `--lib` into three entry
/// points and not the fourth (@PLN120 E.1), and that makes `:reset` silently drop
/// it.  Add a field here and every construction site is a compile error until it
/// is threaded, which is the point.
pub struct ResolutionContext {
    pub stdlib_dir: String,
    pub lib_dirs: Vec<String>,
    /// Registry root, once `loft install` packages join the same path.
    pub registry: Option<String>,
}
```

`ReplSession::new(stdlib_dir)` and `new_with_libs(stdlib, libs)` collapse into
`ReplSession::open(&ResolutionContext)`. The session **keeps** its context, so
`:reset` re-opens with the same one instead of re-deriving it from what happens to be
in scope. `run_loop` takes the context, not `stdlib_dir`.

That converts both remaining bugs into non-events and makes the next flag additive: a
new field is a compile error at all six sites rather than a silent degradation at one.

## Part B — the tooling, which is the actual deliverable

Each item below is tied to a specific blindness this session hit, with the cost.

### B1 — `loft introspect --show-resolution`: make visibility inspectable

**The blindness.** To learn why `hex_distance(…)` would not evaluate, I added an
`eprintln!` to `infer_type` printing `data.source` and `data.source_nr(0..3, name)`,
rebuilt, and read four numbers:

```
EV before: source=0 cur=MAX s0=MAX s1=MAX s2=650 s3=MAX
```

That line *is* the diagnosis — the name lives in source 2, the eval runs in source 0,
and no alias exists anywhere — and it took a compiler edit to see. Nothing in the
tree can answer *"which names can this source see, and where did they come from."*

**The design.** A new `Section::Resolution` beside the existing
`Bytecode`/`Rust`/`Slots`/`Types`/`Ownership` (`src/introspect.rs`), selected by
`--show-resolution`, so it inherits the section plumbing and `--json` for free:

```
=== resolution ===
sources:
  0  <stdlib>            defs 1240   visible 1240
  1  prog.loft   MAIN    defs 3      visible 1247   imports: geom(2) wildcard
  2  lib/geom.loft       defs 2      visible 1242
aliases into source 1 (from imports):
  n_hex_distance      <- source 2  #650  (wildcard use geom)
context: stdlib="default"  lib_dirs=["…/lib"]  registry=none
```

Three properties, each earning its place:

- **`context:`** is Part A's value printed. E.1 becomes visible without running
  anything: `lib_dirs=[]` under a `--lib` invocation is the whole bug, on screen.
- **the alias list** is what E.4 destroyed. A rebuild that drops it shows as an empty
  section, so the defect is *readable* rather than inferred from a failing call.
- **`visible` vs `defs`** separates "defined here" from "reachable here", which is the
  distinction `def_nr` implements and no existing dump exposes.

Plus the question a user actually has, `--why <name>`:

```
$ loft introspect prog.loft --why hex_distance
n_hex_distance  is defined in source 2 (lib/geom.loft), pub
  visible in source 2 (its own)
  visible in source 1 (alias, via `use geom;`)
  NOT visible in source 0 (<stdlib>) — imports do not flow to the stdlib
```

That answers the consumer's report directly, and it answers it from the *shipped*
binary rather than from a build with prints in it.

### B2 — apply the oracle that already exists to the site that needed it

**The finding that should sting.** `Data::derived_indices_diff` exists, and its
doc-comment names E.4's exact class:

> *"any binding the fresh parse holds that the rebuild can't reproduce — **a
> cross-source `def_names` entry**, the `use_names` module map — is a silent
> round-trip gap"*

It is `#[cfg(test)]` and pointed at the **cache** round-trip only. `rebuild_indices`
has two callers; the oracle was aimed at one of them and E.4 lived in the other.

**The design.** In debug builds, have the rollback path assert the same property:
snapshot the derived indices before `rollback_to`, rebuild, and diff. Any binding the
rebuild cannot reproduce becomes a test failure at the moment it is introduced instead
of a consumer report months later.

Note what this is *not*: a new mechanism. It is the existing oracle applied to the
second caller — which is why it is cheap, and why not having done it is the
interesting part. Whenever a check is written for one caller of a shared routine,
the question "which other callers need it" is the whole of the work.

### B3 — a cache must not blind the compiler-debug loop

**The blindness, and it cost the most.** My parser instrumentation produced 93 trace
lines on a virgin directory and **zero** on the same package a second time. Deleting
the package's `.loft/` did not help; only a brand-new path did. For several rounds I
was reading a stale parse and drawing conclusions from it.

**The mechanism is exact, and the guard already exists but misses.**
`cache::cache_decision` disables the cache when `CARGO_MANIFEST_DIR` is set, and its
own comment says why: *"This keeps the **compiler-debug loop** (dev-safety caveat) …
from writing/reading bundles."* `cargo run` sets that variable. Running
`./target/debug/loft` directly — which is what you actually do when iterating on the
compiler, because it skips the rebuild check — does **not**. So the protection aimed
at exactly this situation misses its most common form.

**Two changes, and both are small:**

1. **Widen the signal from the environment to the binary.** Treat a `current_exe()`
   under a `target/debug/` or `target/release/` tree as the compiler-debug loop. The
   fact "this is a development build" is a property of the binary, not of who invoked
   it, so read it there.
2. **Make a served bundle audible when the user is instrumenting.** If any
   `LOFT_LOG` / `LOFT_*` diagnostic variable is set and a cached bundle is used, say
   so on stderr once: `loft: served a cached bundle for prog.loft (LOFT_NO_CACHE=1 to
   re-parse)`. A cache that is silent while you are debugging the parser is a
   blind-instrument generator — and the kill switch existing does not help someone
   who has no reason to suspect a cache.

The general rule, worth stating once: **any layer that can serve a stale answer must
be loud whenever a diagnostic is armed.** Silence is affordable only when nobody is
looking.

## Falsification probes (design-protocol step 3)

Each load-bearing claim, with the cheapest test that could prove it false:

| claim | probe | falsified if |
|---|---|---|
| One context value closes both sites | grep for `ReplSession::new`/`new_with_libs` after the change | any construction site can still omit a field |
| `loft repl --lib` is genuinely broken today | run it — **done**, answers `Library 'geom' not found` | it resolves |
| `:reset` drops libraries | **not runnable today** — latent behind site 1; proven by construction instead, and becomes runnable the moment site 1 is fixed | after site 1, the call still resolves post-`:reset` |
| B1 would have shown E.4 | run `--show-resolution` on the pre-fix binary | the alias list is present, i.e. the section shows nothing useful |
| B2's oracle catches E.4 | disable the E.4 replay, run the rollback assert | it stays green |
| B3's rule explains the blinding | **run — confirmed**: `CARGO_MANIFEST_DIR` is unset for a directly-invoked `target/debug/loft`, so `cache_decision` returns *on*, and a bundle appears under the **library's** `.loft/cache/` (e.g. `tictactoe_client_v2-b79f8d…`) | the variable were set, or no bundle were written |

The fourth row is the one to run **first**: if `--show-resolution` on a pre-E.4 binary
does not visibly differ from a post-fix one, B1 does not earn its place and the design
is wrong about what it would have saved.

## Rejected

- **Fix `loft repl --lib` and `:reset` in place, without the context value.** Two
  three-line changes, and it leaves the shape that produced them — the seventh site
  will be wrong too. This is the "thread the fix through all N sites" conclusion that
  the alarm in step 2 exists to gate.
- **Make `reset()` preserve imports instead of retaining and replaying them.** Tempting
  after E.4, but `reset` means "a fresh parse", and a fresh parse legitimately declares
  its own `use`s. Retaining the *applied* list and replaying after a rebuild keeps the
  two meanings separate; conflating them would make a genuinely fresh parse inherit a
  previous program's vocabulary.
- **A `LOFT_LOG=resolution` trace instead of an introspect section.** A trace shows
  events; the question here is about *state* ("what can this source see"), which wants
  a dump. `LOFT_LOG` is right for a stream and wrong for a table.
- **Disable the program cache by default.** It is a 3–3.6× warm-start win and correct
  in normal use. The defect is that its dev-loop exemption misses a case, not that the
  cache is wrong.

## Steps

Ordered; each is verifiable alone, and B-items land before A so the fix can be *seen*
to work rather than only asserted.

1. `[✓]` **B3** — widen `cache_decision`'s dev-loop signal to the binary's path; announce a
   served bundle when a diagnostic is armed. *Gate:* two consecutive
   `target/debug/loft` runs on a package with a **library** both re-parse (the bundle
   is written per-library, not for a bare single file — a single-file probe would be
   vacuous here); plus a unit test on `cache_decision`'s new input, which is why the
   decision was factored into a pure function in the first place.
2. `[✓]` **B1** — `Section::Resolution` + `--show-resolution` + `--why <name>`. *Gate:* on a
   `--lib` program it prints the alias and the context; with `--lib` omitted it prints
   `lib_dirs=[]`, i.e. E.1 visible without running.
3. `[✓]` **B2** — apply the rollback guard (see § What the build corrected — NOT `derived_indices_diff`, which is a whole-`Data` diff and would drown in a truncation's legitimate differences) at the rollback rebuild under
   `debug_assertions`. *Gate:* the E.4 non-vacuity probe (disable the replay) must now
   fail *at the rebuild*, not at a consumer-level eval.
4. `[✓]` **A** — `ResolutionContext`; `ReplSession::open`; sessions keep their context;
   `:reset` re-opens with it. *Gate:* `loft repl --lib d` resolves a library call, and
   a `:reset` session still resolves one afterwards — the two silent sites, each as an
   assertion.

## What it would have saved, measured

Not a guess — this session's own cost. E.4 took two hand-written instrumentation
rounds plus roughly six turns lost to the cache serving stale parses; B1 answers it in
one command and B3 removes the stale-parse round entirely. E.1 was found by a consumer
filing a report titled *"the debugger does not work on real programs"*; B1 prints
`lib_dirs=[]` for it. And B2 is the cheapest of the three because it is an existing
check applied to a second caller.

## What the build corrected

Three things, and the two that matter were caught by the probes this design named
rather than by re-reading it.

**1. B1 does NOT catch E.4 — the gating probe fired.** The design claimed *"a rebuild
that drops the alias list shows as an empty section"*, and named that as the probe to
run first. It was run first, and it falsified the claim: with `replay_imports`
disabled, `--show-resolution` printed the alias unchanged. The reason is structural —
`introspect` performs one fresh parse and never rolls back, and E.4 is a
rollback-only defect, so no static dump can show it. B1's value is therefore E.1 (the
`context:` line, verified: `lib_dirs=[]` with the flag omitted) and answering
`--why` for a user; **E.4 is B2's alone.** Had the probe been skipped, the design
would have shipped with a false claim about its own coverage.

**2. B2's guard was dead where it was written.** `#[cfg(debug_assertions)]` is a no-op
inside this library: `[profile.dev.package.loft]` sets `debug-assertions = false` (the
store hot-path guards cost ~270×), and that applies under `cargo test` too — so a
cfg-gated assert would run *only* in a release-DA build. The project's own convention,
stated in that same Cargo.toml comment, is that load-bearing checks are plain
`assert!`s. The guard is now unconditional, with its cost bounded by an
`applied.is_empty()` gate so a program with no `use` pays one check.

**3. The guard could not explain itself — E.3's defect at two more sites.** Once the
assert fired, its message vanished: `debug_eval_fmt` wrapped both eval attempts in
`catch_unwind(…).unwrap_or(None)`, and the REPL's run path printed a fixed
*"runtime error (session preserved)"*. Both discarded the panic payload — exactly what
@PLN120 E3a fixed for the debug-abandon path, still live in three more places. Both now
carry the cause (`eval_or_report_panic` routes it through arc B's `trace_output`), which
is what turned the guard from a silent abort into:

```
runtime error: assertion `left == right` failed: a rollback dropped the import alias
`n_hex_distance` visible from source 1 (definition #650) …
```

A named assert that cannot reach the user is not a guard. Worth generalising: **every
`catch_unwind` that discards its payload is a diagnosis destroyed**, and this repo now
has four such sites fixed and none known remaining on the debug paths.

**Also learned, and load-bearing:** `replay_imports` uses `mem::take`, which drains the
retained list — safe only because `import_all`/`import_name` re-`remember` each entry as
the replay runs, refilling it for the *next* rebuild. Verified with three library calls
in one session (a single call would not have shown it). That is commented at the
function, since a reader would otherwise "simplify" the re-record away and break every
rebuild after the first.

## See also

- [`../120-debugger-shape/DESIGN.md`](../120-debugger-shape/DESIGN.md) § E.1, § E.4 —
  the two shipped fixes and their verification.
- `src/cache.rs::cache_decision` — the dev-loop exemption B3 widens.
- `src/data.rs::derived_indices_diff` — the oracle B2 re-points.
- `src/introspect.rs::Section` — where B1's section joins.
