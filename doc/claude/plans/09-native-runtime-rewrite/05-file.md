# Phase 05 — File emitters

**Status:** OPEN

**Closes:** **P200** (binary file `f += <int>` width mismatch — write
side), **P203** (file handle not flushed/closed on `{...}` block
exit).

**Reproducers:** `tests/scripts/repro_p203.loft`,
`tests/docs/13-file.loft`, `tests/scripts/20-binary.loft`.

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

### P203 — root cause beyond the symptom

The interpreter flushes file refs on scope exit; native emits
`OpFreeRef` without distinguishing file refs (need `flush+close`)
from heap refs (just reclaim).  Flavour info exists in the IR
(`Type::Reference(def, …)` points to a `Definition` whose store
kind is the file store) but the emission site doesn't currently
walk that link.

## Prior attempts

- **P200 (write)**: skip-narrow-cast-for-reading-file-blocks fix
  reverted because read-side roundtrip regressed.  Lesson: per-site
  fix is too narrow when the cast helper is shared.
- **P203**: no prior fix attempt recorded.  The diagnostic blocker
  is locating where flavour info should live.

## Why this works now

- **For P200**: phase 02's adapter split + this phase's custom
  emitter together mean the write-Op never enters
  `narrow_int_cast`.  The dual-role conflict is structurally absent.
- **For P203**: phase 00's `EmitCtx` is the home for the
  `is_file_ref(value)` helper.  The helper walks `Type::Reference`
  → `Definition` → store-kind tag.  Pre-work (step 5.1) verifies
  reachability before writing the emitter.

## Detailed steps with validation

### Step 5.1 — Pre-work: verify P203 flavour info is reachable

**Action**: write a one-shot diagnostic script that, given a
`Value::Var(n)` for a file ref, walks the IR to its `Definition`
and reports whether the file-store tag is recoverable.

```rust
// tests/codegen_diagnostic.rs
#[test]
#[ignore]  // run manually as: cargo test --release --test codegen_diagnostic -- --ignored
fn p203_file_flavour_is_reachable() {
    // Compile repro_p203.loft to IR (without emit).
    let data = parse_to_ir("tests/scripts/repro_p203.loft");
    // Locate the OpFreeRef node for the file variable.
    let free_ref_target = find_free_ref_in_block(&data, "f").unwrap();
    // Try to derive store-kind from the target.
    let kind = walk_to_store_kind(&data, &free_ref_target);
    assert!(kind.is_some(), "file-store flavour not reachable from OpFreeRef target");
    assert_eq!(kind.unwrap(), StoreKind::File);
}
```

**Validation**:
```bash
cargo test --release --test codegen_diagnostic -- --ignored p203_file_flavour_is_reachable
```

**Outcomes**:
- Test passes → flavour info is reachable; proceed with the
  emitter as planned.
- Test fails → flavour info is genuinely lost.  Do NOT write the
  emitter; instead reroute P203 to a separate parser/IR-level fix
  and remove P203 from this phase.

### Step 5.2 — Pre-work: pin the prior P200 failure mode

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

### Step 5.3 — Add `EmitCtx::is_file_ref(value)` helper

**Action**: extend `EmitCtx` (from phase 00) with:
```rust
impl<'a, W: Write + ?Sized> EmitCtx<'a, W> {
    pub fn is_file_ref(&self, v: &Value) -> bool {
        let ty = self.value_type(v);
        match ty {
            Type::Reference(def_nr, _) => {
                let def = self.data.def(def_nr);
                def.store_kind == StoreKind::File
            }
            _ => false,
        }
    }

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
fn is_file_ref_true_for_file_handle() {
    let ctx = ctx_for("tests/scripts/repro_p203.loft");
    let v = ctx.local("f");
    assert!(ctx.is_file_ref(&v));
}

#[test]
fn is_file_ref_false_for_heap_ref() {
    let ctx = ctx_for("tests/scripts/struct_basic.loft");
    let v = ctx.local("s");  // a struct local
    assert!(!ctx.is_file_ref(&v));
}

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
cargo test --release --test codegen_emitter::is_file_ref_true_for_file_handle
cargo test --release --test codegen_emitter::is_file_ref_false_for_heap_ref
cargo test --release --test codegen_emitter::int_width_for_returns_field_width
```

### Step 5.4 — Implement `OpWriteIntFile` emitter

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

### Step 5.5 — Implement `OpFreeRef` file-flavour emitter

**Action** (only if step 5.1 passed): create
`src/generation/ops/op_free_ref.rs`:
```rust
pub struct Emitter;

impl OpEmitter for Emitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, dyn Write>, args: &[Value]) -> io::Result<()> {
        let [target] = args else { panic!("OpFreeRef arity") };
        if ctx.is_file_ref(target) {
            write!(ctx.w,
                "{{ stores.flush_file({0}); stores.close_file({0}); stores.free_ref({0}) }}",
                ctx.emit(target)?)?;
        } else {
            crate::generation::ops::default::DefaultTemplateEmitter.emit(ctx, args)?;
        }
        Ok(())
    }
}
```

Register it.

**Validation**:
```bash
# P203 reproducer now passes:
cargo run --bin loft --release -- tests/scripts/repro_p203.loft
test $? -eq 0

# Heap ref free still works (regression guard):
cargo test --release --test wrap struct
cargo test --release --test wrap vector
cargo test --release --test issues 2>&1 | tail -3

# Specific test: write a file, exit block, re-open, assert content:
cat > /tmp/p203_verify.loft <<'EOF'
fn main() {
    {
        let f = file_open_write("/tmp/p203_verify.txt");
        f += "hello";
    }  // block exits — file should flush+close here
    let g = file_open_read("/tmp/p203_verify.txt");
    let content = g.read_text();
    assert(content == "hello", "block-exit flushed content to disk");
}
EOF
cargo run --bin loft --release -- /tmp/p203_verify.loft
test $? -eq 0
```

### Step 5.6 — Update PROBLEMS.md

**Action**: mark P200 (write side) and P203 CLOSED with "fix path:
phase 05 of plan 09".  Reference the regression tests added in
this phase.

**Validation**: review.

## Acceptance for phase 05 overall

```bash
cargo test --release --test codegen_emitter::p200_round_trip_test_compiles_and_runs
cargo test --release --test wrap file
cargo test --release --test wrap binary
cargo test --release --test wrap struct                   # heap-ref free still works
cargo test --release --test wrap vector
cargo test --release --test issues 2>&1 | tail -3         # 540/540
cargo run --bin loft --release -- tests/scripts/repro_p203.loft
cargo run --bin loft --release -- /tmp/p203_verify.loft
cargo test --release --test native -- --test-threads=1 2>&1 | grep "native result"
```

## Commit shape

6-7 commits across the steps; ships as one PR.

## Diagnosis findings

_(populate during pre-work; document the failing test's IR shape,
where flavour info lives, and how the planned fix avoids the prior
failure mode)_

## Problems encountered

_(append per problem)_

## Implementation notes

_(append per non-obvious decision)_
