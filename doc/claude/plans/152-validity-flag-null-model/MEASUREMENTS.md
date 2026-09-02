<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN152 measurements — phases A and B

The numbers behind [README.md](README.md)'s status. Probes: [`probes/`](probes/).

## Phase A result — measured 2026-09-02

**Verdict: the clean design survives.** The gate was *"under a few percent, build it;
around ten, fall back to computing the flag only where an op can be invalid."*

| bench | flag off | flag on | 2× traffic | flag cost | 2× cost |
|---|---|---|---|---|---|
| `02_sum_loop` (treatment — `AddInt` is the hot op) | 1383 ms | 1390 ms | 1405 ms | **+0.51 %** | +1.59 % |
| `01_fibonacci` (treatment — `MinInt`/`AddInt` + calls) | 12581 ms | 12678 ms | 12846 ms | **+0.77 %** | +2.11 % |
| `08_word_count` (**control** — text scanning) | 332 ms | 332 ms | 337 ms | **0.00 %** | +1.51 % |

Medians, `--interpret`, release build, 15 interleaved reps (5 for fibonacci). That is
**0.35 ns/op ≈ 1.1 cycles at 3 GHz** over `02_sum_loop`'s 20 M `AddInt` — physically
credible for three L1-resident, pipelined byte accesses, which is the check that the
number is a measurement rather than an artefact.

**The mechanism is confirmed in the disassembly, not just the timing.** In the flag arm
the operand sentinel tests are *gone* — baseline's `cmp $0x8000…,%rdx; je` (with `v1`'s
test folded into `neg; jo`) is replaced by `add (%rax),%rbx; seto %al`. The plan's claim
that *"the flag is a bit the CPU already sets"* is `seto`, at the instruction level.

**The method matters more than the number, and phases C/D inherit it.** The first three
attempts all failed, each for a different reason worth recording:

1. **A shared box.** Another checkout ran `rustc` + six `mold` linkers mid-run; load hit
   56 and `08_word_count` read 1017, 602, 457, 361, 340, 332, 310 across seven reps — a
   monotone settle, not a measurement.
2. **Separate binaries are not comparable at this effect size.** A build whose only change
   was *adding the shadow field* — ops untouched, the new code dead — moved every
   benchmark by **+2 to +4 %**, treatment and control alike. Later, a flag build made the
   text control **2.6 % faster**, which no change to integer arithmetic can do. Adding
   24 bytes to `State` shifts every field offset and the whole code layout, and that
   swamps what is being measured.
3. **So the measurement must be ONE binary with a runtime-selected path** — identical
   struct, identical layout, only the executed instructions differ. That is where the
   numbers above come from, and it is how C and D must compare before/after too.

The instrument was checked in both directions rather than trusted: the **control must not
move** (0.00 %) and a **double-traffic mode must** (+1.5 to +2.1 %). Both held. The
double-traffic arm scaled 2.7–3.1× rather than a clean 2×, and the control moved 1.5 % in
that arm, which bounds the residual noise at ~1.5 % — so the flag cost is quoted as
*under 1 %*, not to two decimals.

**What this does NOT measure — the honest residual.** The probe carried the flag through
`add` / `sub` / `mul` on integers. A full implementation also needs every *value-producing*
op to write its slot's flag (one store), because the shadow is indexed by stack position
and would otherwise read a stale entry. Scaling from the measured per-access cost, that is
roughly another **0.6–1 %**, putting a complete implementation near **1.5–2 %** — still
well inside the gate, but it is an estimate, and Phase C's parallel run is where it becomes
a measurement.

Probe artefacts are throwaway and were reverted; `src/fill.rs` is generated, so the real
change belongs in the `#rust` templates and the generator, not in that file.

## Phase B result — measured 2026-09-02

Three instruments, each with its own control, in [`probes/`](probes/). The value matrix
runs under `scripts/probe-matrix`, which enforces hand-written `@EXPECT`s, rejects vacuous
cells, and requires a control that FAILS.

### What the matrix says

**19 value cells, both backends, all green; control red; no vacuous cells.** Expectations
were hand-derived from `formal/types.md`'s spare-code table *before* the first run.

- **The value collapse is already uniform across every boundary.** Nine cells put the same
  fault into a local, a struct field, a vector element, a call argument, a return value and
  a tuple element — all answer identically. So `(E-Collapse)` **describes** today's value
  behaviour rather than changing it, which makes Phase D a genuine no-op and its
  identical-output gate both cheap and meaningful. That was the plan's biggest unknown.
- **The divergence is entirely along the TYPE axis**, not the boundary axis: `integer` and
  `i32` answer `null`; `u8`/`i8`/`u16`/`i16` (range fills the width) and `u32` (spare code
  at the top, which no non-null read tests) answer the type's default.

### The residual has exactly ONE door

A five-cell refusal probe, with a control that must compile:

| fault reaching a non-null `u8` | outcome |
|---|---|
| `/` by zero | **refused** — *cannot cast a possibly-null `integer?` to the non-null `u8`* |
| out-of-range index | **refused** — same |
| failed text parse | **refused** — `error[text-parse-may-fail]` |
| out-of-range narrowing cast | **refused** — *use `u8?` for a checked cast* |
| **overflow** (the control) | **compiles, answers `0`** |

Every fault that types `τ?` is stopped by `(N-Store)`/`(N-Cast)` before it can reach a slot
that cannot represent its answer. **Overflow is the only one that gets through, and it gets
through precisely because C85 types it non-null.** So C85's exception is not merely an
inconsistency in the rule set — it is the single reason the unrepresentable case is
reachable at all. That is the sharpest argument for this plan, and it was not known before
this phase.

### The diagnostic channel is NOT uniform

Nine cells, scored by severity and code (a coarse "any diagnostic fired" test scored a
`dead-assignment` as a pass and hid both findings below):

- `(N-Store)` warns at the **field, element, call-argument, return, tuple and keyed**
  boundaries — uniformly.
- A **declared local is a hard ERROR**, not a warning (`(N-Decl)`'s commitment). So the
  boundary set is uniform in VALUE and split in SEVERITY, and `(E-Collapse)` must say so
  rather than implying one answer.
- The **narrow-width overflow collapse is silent** — the case row 3 of the table proposes
  to report.

### The correction this phase forces

**All eight issues this plan cited are `fixed-pending-merge` on this branch**, and loft#1296's
fix (`c75dce65`, the same morning) is what my expectations were derived from: it made `i32`
answer `null` and *added the spare-code table to the rules*. So:

- the plan's four-answer table is post-fix and accurate, but
- the **residual is narrower than the issue body claims**: `u8`/`i8`/`u16`/`i16`/`u32`, and it
  is a documented representation limit rather than an open defect. This plan does not "fix
  loft#1296" — it makes the limit **speak**;
- loft#1284 (the tuple `(N-Store)` gap) is fixed here, which my `d06` cell independently
  re-verified.

**So the bug-fixing case for this plan is largely already banked by other work.** What
survives untouched is the structural case: three encodings of one question, C85's exception
as the single reachability door, the 14 `Op*Nullable` duplicates, the `_nn` lattice this
makes unnecessary, the silent narrow-width default, and provenance. That is still worth the
work — but it is a *consolidation* argument now, not a defect-count argument, and the issue
body should be corrected to say so.

### Not graduated yet, deliberately

The boundary cells pin behaviour that was never broken, so `make falsify` would report them
INERT — there is no commit they would fail on. They are the **before-half of Phase D's
parallel run**, not a regression guard, so they graduate to `tests/scripts/` in D with a real
falsification answer, rather than landing now as an inert guard.

