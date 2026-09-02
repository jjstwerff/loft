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


## Phase C result — measured 2026-09-02

**Phase C as cut is the wrong phase, and running it is what showed that.** The finding is
better than the phase: the collapse site already exists on both backends, and it carries
exactly the defect the proposed rule predicts.

### Native has no eval stack, so the flag has no native analogue

`loft introspect` on a minimal add:

```rust
fn n_add2(cell: …, mut var_a: i64, mut var_b: i64) -> i64 {
  return ops::op_add_int((var_a), (var_b))
}
```

Direct Rust over real locals. Phase A's shadow-array-indexed-by-`stack_pos` has nothing to
attach to here — native would have to thread `(value, ok)` pairs through generated
expressions, and the `#rust` template is an *expression* pasted into arbitrary positions, so
a template returning a pair cannot be substituted where a scalar is expected. **The two
backends need two different mechanisms**, which the phase's one-line description hid.

### But the collapse site already exists — and already reports

A narrow-width store lowers to `OpRangeDefault` on BOTH backends, from one `#rust` template
(`default/01_code.loft:158`):

```rust
{ let _rv = @val;
  if _rv == i64::MIN || (_rv >= @lo && _rv <= @hi) { _rv }
  else { s.raise_recoverable(RangeDefaulted { value: _rv, lo: @lo, hi: @hi }); @dflt } }
```

That is `(E-Collapse)` row 3, already built, already reporting — and the message is the one
the plan wanted: *"value 260 is outside the declared range 0..=255, so the slot took its
default instead"*. It is **opt-in behind `--dev-soft-halt`, not absent.** So row 3 is a
policy question (should this raise be default-on?), not an implementation phase.

### And it has exactly the defect the rule predicts

`_rv == i64::MIN` exempts the sentinel **unconditionally**. Whether the sentinel is a legal
answer depends on the TARGET's nullability — `u8?` must keep it, `u8` cannot hold it — and
`OpRangeDefault` is never told which. **The collapse site is missing precisely the input
`(E-Collapse)` branches on.** That is this plan's thesis, confirmed at one line rather than
argued.

### Two defects, from an axis Phase B pinned

Phase B's cells all overflowed to `260` — merely out of range. None landed *exactly on the
sentinel*, which is a different path through the guard. Crossing overflow-shape × storage-kind:

| | local | field | element | return |
|---|---|---|---|---|
| out of range (`+= 10` → 260) | 0 | 0 | 0 | 0 |
| **on the sentinel** (`+= i64::MAX`) | **null** | 0 | 0 | **interpret null / native 0** |

1. **A non-null `u8` LOCAL reads back `null`.** A narrow local is an `i64` until it is
   materialised (`let mut var_x: i64` in the emitted Rust), the guard exempts the sentinel,
   so it survives in the slot. Field, element and return materialise and narrow it to 0. The
   same declared type answers `0` for one overflow and `null` for another, decided by whether
   the result happens to land on the sentinel.
2. **The return boundary DIVERGES between backends.** The interpreter returns
   `integer(0,255)` as an 8-byte slot (`Return(value=8)`) and the sentinel survives; native
   declares `-> u8` and emits `(var_x) as u8`, truncating it to `0`. Same IR, two lowerings —
   the divergence class the codegen rule says is itself the bug.

### What this does to the plan

- **C is retired as cut.** The flag is not what the narrow-width case needs; the collapse
  site needs ONE INPUT (the target's nullability). That is a far smaller change than
  threading a pair through two backends.
- **E moves up and shrinks**: give `OpRangeDefault` the target's nullability, and decide
  whether `RangeDefaulted` is default-on. Both defects above close with that one input.
- **What still argues for the flag** is unchanged and unproven-against: removing the operand
  sentinel tests from the hot path (measured in A), collapsing the 14 `Op*Nullable`
  duplicates, and provenance. None of those is a correctness argument any more.

Both reproduce on `main` (e9643ff6), verified against a worktree build — so they are
mainline defects, not this branch's, and both are filed with a verified workaround:
**loft#1305** (the sentinel exemption) and **loft#1306** (the return-width divergence).

Probes: [`probes/axis/`](probes/axis/).
