# Phase 08 — Binary read emitter

**Status:** OPEN

**Closes:** **P200 fully** (binary file read-side width mismatch —
the write side closes in phase 05).

**Reproducer:** `tests/scripts/20-binary.loft` (specifically line 82
"single LE roundtrip"), `tests/scripts/p200_round_trip.loft` (added
in phase 05).

**Depends on:** Phase 02 (param adapter — same dual-role
`narrow_int_cast` issue applies on the read side; phase 02's split
makes the read fix possible without regressing the write side) +
Phase 05 (write-side width fix).

## Diagnosis

The read side of the round-trip is precisely where the *prior* P200
fix attempt failed: a write-side fix changed cast behaviour that
the read side relied on, regressing
`20-binary.loft:82` "single LE roundtrip".

The root cause is the same dual-role `narrow_int_cast` documented
in phase 05.  Phase 02's split is the structural prerequisite.

## Prior attempts

Documented under phase 05's prior-attempts section.

## Why this works now

Phase 02 split parameter adaptation from block-tail-expression
coercion.  Phase 05 demonstrated the pattern on the write side.
Phase 08 mirrors it on the read side: the read emitter emits its
own width-aware `read_exact` + `from_le_bytes` inline and never
enters `narrow_int_cast`.

## Detailed steps with validation

> **Pre-flight (per [forwarding-first recipe](00-scaffold.md#verifying-a-new-op-emitter-the-forwarding-first-recipe))**:
> verify `OpReadIntFile` is not in `dispatch.rs::output_call_inner`'s
> special-case match (`grep -n '"OpReadIntFile" =>' src/generation/dispatch.rs`
> should be empty).  If empty, register a forwarding emitter first;
> if hit, write the real emitter directly absorbing the special-case
> logic.

### Step 8.1 — Pre-work: confirm prerequisites are in place

**Action**:
```bash
cargo test --release --test codegen_emitter::param_adaptation_does_not_route_through_narrow_int_cast
# (phase 02 acceptance test)
cargo test --release --test codegen_emitter::p200_round_trip_test_compiles_and_runs
# (phase 05 regression guard — write side green)
```

**Validation**: both green.  If not, fix the upstream phase first.

### Step 8.2 — Pin the read-side failure mode

**Action**: confirm `tests/scripts/20-binary.loft:82` "single LE
roundtrip" still fails today (after phase 05's write fix is in,
the read side is the remaining broken half):

```bash
cargo test --release --test wrap binary -- "single LE roundtrip"
# Expected: FAIL — proves the read side is the residue.
```

Add a width-matrix regression test:
```rust
// tests/codegen_emitter.rs
#[test]
fn p200_read_widths_round_trip() {
    for width in [8, 16, 32, 64] {
        let test = format!("tests/scripts/p200_read_w{width}.loft");
        // Each test: write fixed value of width W, close, read back, assert eq.
        let status = std::process::Command::new("cargo")
            .args(["run", "--bin", "loft", "--release", "--", &test])
            .status().unwrap();
        assert!(status.success(), "P200 read regressed at width {width}");
    }
}
```

```bash
# Generate the test files:
for w in 8 16 32 64; do
    cat > "tests/scripts/p200_read_w$w.loft" <<EOF
fn main() {
    let f = file_open_write("/tmp/p200_w$w.bin")
    let v: u$w = if $w == 8 { 0x42 }
                 else if $w == 16 { 0x1234 }
                 else if $w == 32 { 0x12345678 }
                 else { 0x123456789ABCDEF0 }
    f += v
    f.close()

    let g = file_open_read("/tmp/p200_w$w.bin")
    let r: u$w = g.read_u$w()
    g.close()
    assert(r == v, "width $w round-trip")
}
EOF
done
```

**Validation**:
```bash
cargo test --release --test codegen_emitter::p200_read_widths_round_trip
# Expected: FAIL (read side still broken).  Will pass after step 8.3.
```

### Step 8.3 — Implement `OpReadIntFile` emitter

**Action**: create `src/generation/ops/op_read_int_file.rs`:
```rust
// Mirrors phase 05's OpWriteIntFile width-aware emission.

pub struct Emitter;

impl OpEmitter for Emitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, dyn Write>, args: &[Value]) -> io::Result<()> {
        let [file, target] = args else { panic!("OpReadIntFile arity") };
        let width = ctx.int_width_for(target);
        let signed = ctx.int_signed_for(target);
        let bytes = (width as usize) / 8;
        let prim = if signed { format!("i{width}") } else { format!("u{width}") };
        write!(ctx.w,
            "{{ let mut buf = [0u8; {bytes}]; {}.read_exact(&mut buf)?; \
             {prim}::from_le_bytes(buf) as i64 }}",
            ctx.emit(file)?)?;
        Ok(())
    }
}
```

Determine the exact Op name from inspecting
`tests/scripts/20-binary.loft` under `--native-emit`.  Register
the emitter.

**Validation**:
```bash
# Width matrix passes:
cargo test --release --test codegen_emitter::p200_read_widths_round_trip

# Original 20-binary suite:
cargo test --release --test wrap binary

# Specifically the historically-broken case:
cargo test --release --test wrap binary -- "single LE roundtrip"

# Endian-correctness sanity:
cargo run --bin loft --release -- tests/scripts/p200_read_w16.loft
xxd /tmp/p200_w16.bin    # expected: bytes "34 12" (LE)
```

### Step 8.4 — Add structural test pinning the dual-role bypass

**Action**:
```rust
#[test]
fn p200_read_emitter_does_not_use_narrow_int_cast() {
    let src = compile_to_rust("tests/scripts/p200_read_w16.loft");
    // Read-Op emission must inline its width-aware decode; not
    // delegate to narrow_int_cast (which is now dual-purpose).
    let read_call_idx = src.find("read_exact").expect("read emitter ran");
    // No narrow_int_cast call within ~200 chars of the read site:
    let surrounding = &src[read_call_idx.saturating_sub(200)..(read_call_idx + 200).min(src.len())];
    assert!(!surrounding.contains("narrow_int_cast"),
        "P200 read path still routes through narrow_int_cast");
}
```

**Validation**: test passes.

### Step 8.5 — Update PROBLEMS.md

**Action**: mark P200 fully CLOSED with "fix path: phase 05
(write) + phase 08 (read) of plan 09".  List all the regression
tests that pin both sides.

**Validation**: review.

## Acceptance for phase 08 overall

```bash
cargo test --release --test codegen_emitter::p200_round_trip_test_compiles_and_runs   # write side
cargo test --release --test codegen_emitter::p200_read_widths_round_trip              # read side, all widths
cargo test --release --test codegen_emitter::p200_read_emitter_does_not_use_narrow_int_cast
cargo test --release --test wrap binary
cargo test --release --test issues 2>&1 | tail -3
cargo test --release --test native -- --test-threads=1 2>&1 | grep "native result"
```

## Gate updates per step

| Step | Gate update |
|---|---|
| 8.2 | Adds width-matrix regression tests (4 widths). |
| 8.3 | `OpReadIntFile` emitter registered.  `custom_count` += 1.  Baseline shape changes for binary-read sites — refresh if corpus references them. |
| 8.4 | New structural test pinning the dual-role bypass. |

## Commit shape

3-4 commits across the steps; ships as one PR.

## Problems encountered

_(append per problem — endian round-trip, partial reads mid-block)_

## Implementation notes

_(append per non-obvious decision)_
