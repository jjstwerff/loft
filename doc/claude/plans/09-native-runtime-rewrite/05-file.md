# Phase 05 — File emitters

**Status:** OPEN

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

### Step 5.0 — Forwarding-emitter smoke test (deferred from phase 00)

**Action**: ship a no-op forwarding `OpEmitter` for `OpWriteIntFile`
as the first commit of this phase.  The emitter does nothing more
than call `DefaultEmitter::emit` (or directly call back into
`Output::user_fn_call_body` / `substitute_template_body`).  Register
it.  Re-run the byte-identical baseline gate.

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
# Currently this should FAIL (P200 still active) — that's the
# regression guard before the fix ships.  Run it as ignored or
# expect-fail until step 5.4.
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

## Commit shape

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

_(populate during pre-work; document the failing test's IR shape
and how the planned fix avoids the prior failure mode)_

## Problems encountered

_(append per problem)_

## Implementation notes

_(append per non-obvious decision)_
