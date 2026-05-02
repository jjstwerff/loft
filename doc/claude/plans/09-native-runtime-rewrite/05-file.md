# Phase 05 — File emitters

**Status:** OPEN — **PLAN MISALIGNED with current errors; rewrite
required before implementation.**  See
[Diagnosis findings](#diagnosis-findings) for details: the failing
sites in `tests/scripts/20-binary.loft` are read-side comparison-
emission errors (E0308 between a narrowed `as u8/u16/u32` LHS and
an `_i64` literal RHS), not the write-side `f += val` template
issue this plan was originally designed for.

**Closes:** **P200** (binary file `f += <int>` width mismatch —
write side).

**Reproducers:** `tests/docs/13-file.loft`,
`tests/scripts/20-binary.loft`.

**Note on P203:** earlier drafts of this phase claimed to close
P203 via a file-flavour `OpFreeRef` emitter.  Plan 10's phase 00
diagnostic refuted that framing — P203 is a template
double-substitution bug, closed structurally by phase 00 step
0.7b's let-bind-on-repeat in the `DefaultTemplateEmitter` (or by
a direct edit to the 5 affected templates in `default/01_code.loft`,
whichever lands first).  Phase 05 now ONLY covers P200's write
side.  The original step 5.1b (verify OpFreeRef fires) is
removed since OpFreeRef firing was never the bug.

**Depends on:** Phase 00 (scaffold) + Phase 02 (param adapter — the
prior direct fix for P200 failed because of dual-role
`narrow_int_cast`; phase 02's adapter split removes that blocker).

## Diagnosis

### P200 (write side) — root cause beyond the symptom

The template hard-codes `i32` for binary writes:
```
#rust"@v0.write_all(&(@v1 as i32).to_le_bytes())?"
```

The deeper issue: `src/generation/emit.rs::narrow_int_cast` has
**dual role** (block-tail-expression coercion AND parameter
narrowing).  A prior session's write-side fix introduced a runtime
failure on the read side at `tests/scripts/20-binary.loft:82`
("single LE roundtrip") because the cast-decision logic was
load-bearing on both paths.

After phase 02, parameter narrowing has its own `NarrowIntAdapter`
in `src/generation/ops/params.rs`; `narrow_int_cast` no longer
serves the param path.  This phase's emitter writes the
width-aware cast inline — bypassing both the template and
`narrow_int_cast` — so neither role of the helper is touched.

## Prior attempts

- **P200 (write)**: skip-narrow-cast-for-reading-file-blocks fix
  reverted because read-side roundtrip regressed.  Lesson: per-site
  fix is too narrow when the cast helper is shared.

## Why this works now

Phase 02's adapter split + this phase's custom emitter together
mean the write-Op never enters `narrow_int_cast`.  The dual-role
conflict is structurally absent.

## Detailed steps with validation

> **Plan rewrite (2026-05-02)**: phase 00a + the actual-error
> survey on `20-binary.loft` showed the failing emission is on the
> READ side (block-tail narrow vs i64 literal RHS), not the write
> side.  The original write-side steps (forwarding emitter for
> `OpWriteIntFile` + `int_width_for` helpers + width-aware
> template) are kept in [§ Historical / write-side scope
> (deferred)](#historical--write-side-scope-deferred) below for
> reference; the active plan is the read-side steps below.  Before
> any code, read `feedback_actual_error_survey.md` — it codifies
> the lesson that prompted this rewrite.

### Step 5.1 — Actual-error survey (per `feedback_actual_error_survey.md`)

**Action**: capture the current state of all P200 failures in the
generated Rust to confirm scope before writing any fix.

```bash
mkdir -p /tmp/p05-survey
cargo run --bin loft --release --quiet -- \
    --native-emit /tmp/p05-survey/20-binary.rs \
    tests/scripts/20-binary.loft
cargo test --release --test native -- --test-threads=1 2>&1 \
    | grep -A20 "rustc failed for 20_binary"
```

Document per failing site:
- Line number in the generated Rust
- LHS expression shape (typically `(var__read_N) as <narrow>` from
  the read block's tail)
- RHS expression shape (typically `<int>_i64`)
- Surrounding `n_assert` / arithmetic context

**Validation**: produces a list of all (site, LHS shape, RHS
shape) tuples.  Confirms whether the failures all share one
shape (E0308 narrow vs i64) or split across multiple shapes.

**Pre-flight**: skip this phase entirely if the survey finds
zero E0308s — the bug may have closed transitively via another
phase.  Surface the unexpected-pass case as a finding rather than
a setup bug.

### Step 5.2 — Identify the comparison-emission code path

**Action**: locate where the comparison `==` (and its siblings
`!=`, `<`, `<=`, `>`, `>=`) get emitted with their LHS / RHS
types decided.  Suspected entry points:

```bash
grep -rn 'OpEqInt\|OpNeInt\|op_eq_int' src/generation/ | head -10
grep -rn 'narrow_int_cast' src/generation/ | head -10
grep -rn '"==" =>' src/generation/ | head -10
```

Document in "Diagnosis findings":
- Which fn emits the LHS (block-tail) — likely
  `src/generation/emit.rs::narrow_int_cast` based on prior P200
  diagnosis.
- Which fn emits the RHS literal — likely a separate path in
  `emit.rs` or the comparison Op template.
- Whether the comparison emitter has access to LHS type info to
  pick a matching RHS suffix.

**Validation**: identify exactly the file:line pair where the
fix lands.  If two candidate sites exist, pick the one that
fixes all 5 surveyed failures uniformly; document the choice.

### Step 5.3 — Pin the prior failure mode via regression test

**Action**: write a regression test that compiles
`tests/scripts/20-binary.loft` under `--native` and asserts
zero rustc errors.  Add to `tests/codegen_emitter.rs`:

```rust
#[test]
fn p200_binary_read_compiles_under_native() {
    let status = std::process::Command::new("cargo")
        .args(["run", "--bin", "loft", "--release", "--",
               "tests/scripts/20-binary.loft"])
        .status().unwrap();
    assert!(status.success(),
        "P200: 20-binary native compile regressed");
}
```

**Validation**: this test currently FAILS (that's the
regression guard before the fix ships).  Commit it as the test
pinning the prior failure mode.

### Step 5.4 — Apply the fix

**Action**: based on step 5.2's identified site, choose between:

**Option A — Drop the block-tail narrow when consumer is `==`
against a fitting constant:**
- Modify `narrow_int_cast` (or its caller) to skip the narrow
  when the block is consumed by a comparison whose RHS is an
  integer literal that fits the narrow type.
- Risk: needs context flow ("what consumes this block") — the
  block-tail emitter today doesn't know its consumer.

**Option B — Widen the constant at comparison-emission time:**
- Modify the comparison emitter (likely an `OpEqInt` template or
  inline emission in `emit.rs`) to inspect LHS type and emit RHS
  as `(<lit>_<narrow>)` instead of `(<lit>_i64)`.
- Risk: needs LHS type info at the comparison site, which may
  also not be readily available.

**Option C (fallback) — Cast both sides to a common width:**
- Wrap the RHS in `(<lit> as <narrow>)` when LHS has narrow
  cast.  Always works; less elegant.

The choice depends on which option's required info is already
available at the relevant emit site.  Implementation step picks
the cleanest of the three based on step 5.2's findings.

**Validation**:
```bash
cargo test --release --test codegen_emitter::p200_binary_read_compiles_under_native
cargo test --release --test native -- --test-threads=1 2>&1 \
    | grep "native result"   # 89/93 → 90+/93 (P200 sub-failure closed)
cargo test --release --test issues 2>&1 | tail -3   # 540/540 unchanged
scripts/p09_fast_gate.sh   # byte-identical (or refresh with intentional change)
```

### Step 5.5 — Update PROBLEMS.md + plan README

**Action**: mark P200 CLOSED (read side) with "fix path: phase
05 of plan 09 (rewritten 2026-05-02 to address read-side
comparison emission)".  Reference the regression test added.
Update plan-09 README to mark phase 05 DONE.  P200's separate
"closure of all binary-write paths" remains in phase 08's scope.

**Validation**: review.

### Historical / write-side scope (deferred)

The original phase 05 steps (kept here for reference; not on
the active critical path) targeted a write-side
`OpWriteIntFile` custom emitter:

#### Step 5.0 (historical) — Forwarding-emitter smoke test (forwarding-first recipe)

**Action**: ship a no-op forwarding `OpEmitter` for `OpWriteIntFile`
as the first commit of this phase.  The emitter does nothing more
than call `DefaultEmitter::emit` (or directly call back into
`Output::user_fn_call_body` / `substitute_template_body`).  Register
it.  Re-run the byte-identical baseline gate.

**Status (2026-05-02)**: this step ALREADY SHIPPED in commit
`a078bac` ("plan-09 phase 05: forwarding-emitter smoke test as
step 5.0").  The forwarding emitter is registered but is a
no-op since `OpWriteIntFile` isn't the actual fix site — the
forwarding emitter remains as a dead-but-harmless smoke test.
A future phase 05b (if write-side issues surface) can replace
its body with real emission logic.

**Pre-flight check** (per the [forwarding-first recipe](00-scaffold.md#verifying-a-new-op-emitter-the-forwarding-first-recipe)):

```bash
grep -n '"OpWriteIntFile" =>' src/generation/dispatch.rs
```

If this returns a hit, `OpWriteIntFile` is in dispatch.rs's
special-case match and the forwarding pattern doesn't apply —
the real emitter (step 5.3) must absorb whatever logic that arm
contains.  If empty, forwarding is safe; proceed.

```rust
// src/generation/ops/op_write_int_file.rs (forwarding stub)
use super::{EmitCtx, OpEmitter, default::DefaultEmitter};
use crate::data::Value;
use std::io;

pub struct Emitter;

impl OpEmitter for Emitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        // Step 5.0 forwarding stub: prove the dispatch flow fires
        // for a registered emitter.  Step 5.4 replaces this with the
        // width-aware emission body.
        DefaultEmitter.emit(ctx, args)
    }
}
```

```rust
// src/generation/ops/mod.rs::build_registry — add:
//     r.insert("OpWriteIntFile", Box::new(super::op_write_int_file::Emitter));
```

**Why**: phase 00's smoke test in `src/generation/ops::tests`
compile-checks the `OpEmitter` trait shape but defers two runtime
verifications because the registry is `OnceLock`-backed and no
`Output` fixture exists in the test surface:

1. **Registry actually fires a registered emitter** — the dispatch
   flow `emit_op → registry().get(name) → emitter.emit()` is
   four lines of code, but never exercised end-to-end.
2. **Emitter can call `ctx.output.<method>` from inside its emit
   body** — the borrow-checker reborrow chain `&mut ctx → &mut
   ctx.output → &mut Output → &mut self` is unproven for any
   real method call.

A forwarding emitter is the cheapest place to validate both:
- Registry fires it (or doesn't, surfacing wiring bugs immediately).
- The forwarding call `DefaultEmitter.emit(ctx, args)` itself is a
  test of `EmitCtx` field access from inside an `OpEmitter::emit`.
- If anything in the dispatch is broken, the byte-identical
  baseline diff fails on this commit, with the smallest possible
  blame surface (one new file, one registry line).

If the byte-identical gate passes after this commit: the trait
dispatch is structurally sound, and step 5.1+ can proceed with
confidence.  If it fails: the diff narrows the bug to the
forwarding wiring.

**Validation**:

```bash
scripts/p09_fast_gate.sh                    # byte-identical, P203 PASS
cargo test --release --lib generation::ops  # smoke tests still pass
cargo test --release --test codegen_emitter # 5/5 (gates)
cargo test --release --test issues 2>&1 | tail -3  # 540/540
```

**Outcome**: confidence that the dispatch surface is live before
step 5.1 starts writing real code.  Cost: ~15 lines of code,
~2 minutes wall time including running the gate.

### Step 5.1 — Pre-work: pin the prior P200 failure mode

**Action**: write a regression test that fails under the prior
reverted fix but passes under the planned emitter.  This pins the
"single LE roundtrip" failure shape.

```rust
// tests/scripts/p200_round_trip.loft  (new — minimal repro)
fn main() {
    let f = file_open_write("/tmp/p200_test.bin");
    let value: u16 = 0x1234;
    f += value;
    f.close();

    let g = file_open_read("/tmp/p200_test.bin");
    let read_back: u16 = g.read_u16();
    g.close();

    assert(read_back == value, "round-trip preserves u16 width");
}
```

```rust
// tests/codegen_emitter.rs
#[test]
fn p200_round_trip_test_compiles_and_runs() {
    let status = std::process::Command::new("cargo")
        .args(["run", "--bin", "loft", "--release", "--",
               "tests/scripts/p200_round_trip.loft"])
        .status().unwrap();
    assert!(status.success(), "P200 round-trip regression");
}
```

**Validation**:
```bash
cargo test --release --test codegen_emitter::p200_round_trip_test_compiles_and_runs
# This test is RED at this commit (P200 still active).  The fix
# in step 5.4 makes it green.  Both commits land in the SAME
# session — the test never sees a green main; it goes from
# "newly added, red" → "fix landed, green" without ever shipping
# in a red state.
```

### Step 5.2 — Add `EmitCtx::int_width_for` / `int_signed_for` helpers

**Action**: extend `EmitCtx` (from phase 00) with:
```rust
impl<'a, W: Write + ?Sized> EmitCtx<'a, W> {
    pub fn int_width_for(&self, v: &Value) -> u8 {
        // Walk the value's resolved Type and return the width in bits.
        // u8 → 8, u16/i16 → 16, …
    }

    pub fn int_signed_for(&self, v: &Value) -> bool { ... }
}
```

Add unit tests:
```rust
#[test]
fn int_width_for_returns_field_width() {
    let ctx = ctx_for("tests/scripts/repro_p200_widths.loft");
    assert_eq!(ctx.int_width_for(&u8_field), 8);
    assert_eq!(ctx.int_width_for(&u16_field), 16);
    assert_eq!(ctx.int_width_for(&i32_field), 32);
}
```

**Validation**:
```bash
cargo test --release --test codegen_emitter::int_width_for_returns_field_width
```

### Step 5.3 — Implement `OpWriteIntFile` emitter

**Action**: create `src/generation/ops/op_write_int_file.rs`:
```rust
// Original #rust template (default/02_images.loft or similar):
//   #rust"@v0.write_all(&(@v1 as i32).to_le_bytes())?"
// Hardcoded i32 — wrong for u8/u16/u64 fields.

pub struct Emitter;

impl OpEmitter for Emitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, dyn Write>, args: &[Value]) -> io::Result<()> {
        let [file, value] = args else { panic!("OpWriteIntFile arity") };
        let width = ctx.int_width_for(value);
        let signed = ctx.int_signed_for(value);
        let prim = if signed { format!("i{width}") } else { format!("u{width}") };
        write!(ctx.w, "{}.write_all(&({} as {prim}).to_le_bytes())?",
            ctx.emit(file)?, ctx.emit(value)?)?;
        Ok(())
    }
}
```

Register it.  Determine the exact Op name from inspecting
`tests/scripts/20-binary.loft` under `--native-emit`.

**Validation**:
```bash
# Regression guard from step 5.2 now passes:
cargo test --release --test codegen_emitter::p200_round_trip_test_compiles_and_runs

# Round-trip suite:
cargo test --release --test wrap binary

# Pin the specific failure case the prior fix broke:
cargo test --release --test wrap binary -- "single LE roundtrip"

# Width matrix — assert each width round-trips:
for w in 8 16 32 64; do
    sed -i "s/let value: u[0-9]*/let value: u$w/" tests/scripts/p200_round_trip.loft
    cargo run --bin loft --release -- tests/scripts/p200_round_trip.loft
    test $? -eq 0 || echo "FAIL at width $w"
done
```

### Step 5.4 — Update PROBLEMS.md

**Action**: mark P200 (write side) CLOSED with "fix path: phase 05
of plan 09".  Reference the regression tests added in this phase.
Note that P200's read side closes in phase 08.  P203 is closed by
phase 00 step 0.7b (let-bind-on-repeat) — not by this phase.

**Validation**: review.

## Acceptance for phase 05 overall

```bash
cargo test --release --test codegen_emitter::p200_round_trip_test_compiles_and_runs
cargo test --release --test wrap file
cargo test --release --test wrap binary
cargo test --release --test issues 2>&1 | tail -3         # 540/540
cargo test --release --test native -- --test-threads=1 2>&1 | grep "native result"
```

## Gate updates per step

| Step | Gate update |
|---|---|
| 5.0 | Forwarding `OpEmitter` registered.  Gate's `custom_count` increments from 0 → 1 (the runtime smoke test).  Byte-identical baseline must stay green. |
| 5.1 | Adds `p200_round_trip` regression test; gate's codegen_emitter test count grows. |
| 5.2 | Adds `EmitCtx::int_width_for` / `int_signed_for` helpers; emitter unit tests added. |
| 5.3 | Replaces step 5.0's stub body with width-aware emission.  Baseline for binary-write tests changes (intentional — width cast inline now).  Refresh baseline if corpus baselines reference `OpWriteIntFile` sites. |
| 5.4 | PROBLEMS.md update; no gate impact. |

4-5 commits across the steps (down from 6-7 after P203 was
removed; bumped to 4-5 by step 5.0's forwarding-emitter smoke test);
ships as one PR.

Recommended commit order:

  1. **Step 5.0**: forwarding `OpEmitter` for `OpWriteIntFile`
     (no-op stub).  Byte-identical baseline must still pass.
     This is the runtime smoke test deferred from phase 00.
  2. **Step 5.1**: pin prior P200 failure mode via regression test.
  3. **Step 5.2**: add `EmitCtx::int_width_for` / `int_signed_for`
     helpers (or expose via `ctx.output`).
  4. **Step 5.3**: replace step 5.0's stub body with the
     width-aware emission.  P200 round-trip test now passes.
  5. **Step 5.4**: update PROBLEMS.md.

## Diagnosis findings

### Scope misalignment surfaced 2026-05-02 — must rewrite plan before implementing

`tests/scripts/20-binary.loft` was selected as the P200 reproducer
under the assumption that the failing emission was on the **write**
side (the `f += val` template hard-coding `i32`).  Inspection of
today's native-emit output for that test shows the failures are
actually on the **read** side:

```rust
// /tmp/loft_native_20_binary.rs:528-532 (representative)
n_assert(cell, {({ //reading file_4: integer(0, 255)
    let mut var__read_3: i64 = i64::MIN as i64;
    OpReadFile(cell, var_f, &mut var__read_3, 1_i64, 11_i32);
    (var__read_3) as u8
} /*reading file_4: integer(0, 255)*/) == (0_i64)},
    "u8 read 0", "tests/scripts/20-binary.loft", 20_i64);
//                                                  ^^^^^^^
// error[E0308]: expected `u8`, found `i64`
```

Five sites fail with the same shape.  The block-tail `as u8`
narrows the read result; the comparison RHS is the integer literal
emitted as `_i64`.  Rust will not auto-coerce one side, so the
`==` is ill-typed.

This is **comparison-emission territory**, not write-side template
territory.  The phase 05 plan as written (custom `OpWriteIntFile`
emitter + `EmitCtx::int_width_for` / `int_signed_for` helpers) does
not address the actual error.  Two candidate fix sites:

1. **Drop the block-tail narrow** when the consumer is `==` against
   a constant whose value range fits the narrow type — let the
   comparison happen at i64 width and trust the read body's range
   to enforce the contract.
2. **Widen the constant** at comparison emission time to match the
   block-tail's narrowed type (`(... as u8) == (0u8)`).  Site:
   wherever `==` is emitted with an integer literal RHS.

Either fix lives in comparison-emission code (probably
`src/generation/emit.rs` or the comparison Op emitter family),
not in `OpWriteIntFile`'s template.  Phase 05 needs:

- New diagnostic step 5.1: **identify the comparison-emission
  site** that picks RHS type.  Confirm the narrowed-block-tail
  pattern from `narrow_int_cast` is the LHS shape.
- Revised step 5.3: **emit the comparison with matched widths**
  rather than building a width-aware OpWriteIntFile emitter.
- Phase 02 (param adapter) prerequisite no longer obvious — the
  read-side narrow comes from the block-tail role of
  `narrow_int_cast`, not the param role.  Phase 02's split of
  the helper might still help (cleaner code), but is no longer
  on the P200 critical path.

The original phase 05 design (write-side `OpWriteIntFile` emitter)
might still be needed if a separate write-side error surfaces in
20-binary or other tests after the read-side fix.  Treat it as a
phase 05b extension, not the main scope.

**Action before phase 05 starts**:
1. Re-read all 5 failing sites in `/tmp/loft_native_20_binary.rs`;
   confirm read-side comparison is the only error class.
2. Trace the comparison-emission code path that picks the RHS
   literal's type tag (`_i64`).
3. Rewrite phase 05's "Detailed steps" + "Why this works now" to
   match the actual fix site.
4. Re-evaluate whether phase 02 is still listed as prerequisite.

_(populate during pre-work; document the failing test's IR shape
and how the planned fix avoids the prior failure mode)_

## Problems encountered

_(append per problem)_

## Implementation notes

_(append per non-obvious decision)_
