# Phase 02 — Loosen the scope-exit emission gate

**Status:** OPEN

**Closes:** **P203** (file handle not flushed on `{...}` block
exit).  Likely also closes the cleanup-side of **P204** (work-ref
unfree) but the Call-resolution side stays open.

**Reproducer:** `tests/scripts/repro_p203.loft`.

**Depends on:** Phase 00 (survey identifies the exact gating
condition that fails for P203) + Phase 01 (runtime safety tests
pin the invariants this phase relies on).

## Diagnosis

### Symptom
`tests/scripts/repro_p203.loft` PASSES under `--interpret`, FAILS
under `--native`.  Block-scoped file ref `f` doesn't flush/close
on block exit; subsequent `delete()` panics.

### Root cause (per phase 00 step 0.3)
The scope-exit walk in `src/scopes.rs::get_free_vars` already
iterates over all locals.  The gate at line 1053 decides per-var
whether to emit OpFreeRef:

```rust
let emit = (dep.is_empty() || is_work_ref) && !in_ret && !function.is_skip_free(v);
```

For the file ref `f` declared inside the `{...}` block, one of
the negative conditions evaluates true (most likely
`dep.is_empty() == false` — the dep tracker incorrectly attaches
a non-empty dep list to the file ref).  No OpFreeRef is emitted;
the file never closes.

### Why this works
The fix isn't to debug the dep tracker — that's the precise
mechanism that's been brittle across attempts.  Instead, **loosen
the gate** so file refs (and possibly all heap-typed refs)
always emit OpFreeRef regardless of dep state.  Phase 01's
runtime safety contract makes the extra emissions safe (already-
freed slots no-op; non-aliased refs free correctly).

The dep mechanism stays for OTHER consumers (aliasing analysis,
closure capture, parallel isolation per phase 00 step 0.4); only
its role as the cleanup-emission gate is retired.

## Detailed steps with validation

### Step 2.1 — Read the gate site precisely

**Action**: `src/scopes.rs:1001-1090` is the heap-ref scope-exit
emission block.  Re-read it with phase 00 step 0.3's findings in
hand.

The current gate (line 1053):
```rust
let emit = (dep.is_empty() || is_work_ref) && !in_ret && !function.is_skip_free(v);
```

Identify which condition fails for the P203 file ref.  Phase 00
step 0.3 should have answered this; if not, instrument with
`scope_debug` and run the reproducer first.

**Validation**: written summary of the failing condition, used to
plan the exact edit in step 2.2.

### Step 2.2 — Loosen the gate for ref-shaped types

**Action**: change line 1053 from:

```rust
let emit = (dep.is_empty() || is_work_ref) && !in_ret && !function.is_skip_free(v);
```

to:

```rust
// Plan 10: scope-walk emits OpFreeRef for any ref-shaped local
// at scope exit unless the var participates in the return path
// or is explicitly marked skip_free.  The runtime no-ops on
// already-freed slots (see runtime safety contract in
// LIFETIME.md), so the dep gate is no longer load-bearing for
// correctness — it was an optimisation that masked emission
// gaps (P203/P204 cleanup-side).
let emit = !in_ret && !function.is_skip_free(v);
```

This is a **3-character functional change** (delete the `(dep.is_empty()
|| is_work_ref) && ` prefix).  All other gating stays.

**Validation**: this is the load-bearing single edit.

```bash
cargo build --release
# Compiles: yes (no API change).

# repro_p203 now exits 0 under native:
cargo run --bin loft --release -- tests/scripts/repro_p203.loft
echo "Exit: $?"
# Expected: 0.  File present on disk after block exit.
test -f /tmp/repro_p203.txt && echo "FILE PRESENT" || echo "FILE MISSING"

# Full suite must stay green:
cargo test --release --test issues 2>&1 | tail -3            # 540/540
cargo test --release --test threading 2>&1 | tail -3         # 43/43
cargo test --release --test threading_chars 2>&1 | tail -3   # 35/35
cargo test --release --test native -- --test-threads=1 2>&1 | grep "native result"
```

**If full suite stays green** → proceed to step 2.3.

**If any test regresses** → identify which test, classify:
- Test relied on a specific OpFreeRef *not* firing → add an
  exception in step 2.3 via `is_skip_free`.
- Test exposed a real bug the old gate was masking → diagnose
  separately; do not paper over.

### Step 2.3 — Audit regressions and update suppression list

**Action**: for each regression from step 2.2, classify and
handle:

```bash
cargo test --release 2>&1 | grep -E "FAILED" | tee /tmp/p10-regressions.txt
```

For each failing test:

1. Identify which var(s) the new emission frees that shouldn't be.
2. Check whether the var has a deliberate "owned by callee"
   semantics (the callee runs OpFreeRef on it).
3. If yes → mark the var with `set_skip_free` at the point where
   it's passed to the owning callee.
4. If no → diagnose the regression; the gate loosening exposed a
   real bug.

```rust
// In src/parser/control.rs or similar, where the callee-takes-
// ownership pattern is recognised:
function.set_skip_free(callee_owned_var);
```

**Validation**: full suite green after suppression-list updates.

```bash
cargo test --release --test issues 2>&1 | tail -3
cargo test --release --test threading 2>&1 | tail -3
cargo test --release --test threading_chars 2>&1 | tail -3
cargo test --release --test native -- --test-threads=1 2>&1 | grep "native result"

# All previously-failing suite slots now pass.
```

### Step 2.4 — Add P203 regression test

**Action**: append to `tests/runtime_safety.rs`:

```rust
#[test]
fn p203_file_block_close_flushes_native() {
    // Run the reproducer; expect exit 0.
    let status = std::process::Command::new("cargo")
        .args(["run", "--bin", "loft", "--release", "--",
               "tests/scripts/repro_p203.loft"])
        .status().unwrap();
    assert!(status.success(), "P203 reproducer fails under native");
}

#[test]
fn p203_file_block_close_writes_to_disk() {
    let path = "/tmp/p203_block_disk.txt";
    let _ = std::fs::remove_file(path);
    let test = "tests/scripts/p203_block_disk.loft";
    std::fs::write(test, &format!(r#"
fn main() {{
    {{f = file("{path}"); f += "x"}}
}}
    "#)).unwrap();
    let status = std::process::Command::new("cargo")
        .args(["run", "--bin", "loft", "--release", "--", test])
        .status().unwrap();
    assert!(status.success());
    let content = std::fs::read_to_string(path).expect("file must exist on disk after block exit");
    assert_eq!(content, "x");
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(test);
}
```

**Validation**:
```bash
cargo test --release --test runtime_safety::p203_file_block_close_flushes_native
cargo test --release --test runtime_safety::p203_file_block_close_writes_to_disk
```

### Step 2.5 — Add structural test pinning the new contract

**Action**:
```rust
// tests/runtime_safety.rs
#[test]
fn scope_exit_does_not_depend_on_dep_being_empty() {
    // Compile a test where the file ref has a non-empty dep
    // (the parser attaches a dep when the ref originates from
    // a function call returning a ref).  The generated code
    // must STILL contain OpFreeRef for that local at scope exit.
    use std::process::Command;
    let test = "tests/scripts/p10_dep_nonempty.loft";
    std::fs::write(test, r#"
fn make_file(path: text) -> file { file(path) }
fn main() {
    {f = make_file("/tmp/p10_dep.txt"); f += "y"}
}
    "#).unwrap();

    let out = "/tmp/p10_dep.rs";
    let status = Command::new("cargo")
        .args(["run", "--bin", "loft", "--release", "--quiet", "--",
               "--native-emit", out, test])
        .status().unwrap();
    assert!(status.success());

    let src = std::fs::read_to_string(out).unwrap();
    assert!(src.contains("OpFreeRef"),
        "phase 02 contract: scope-walk emits OpFreeRef regardless of dep state");
    let _ = std::fs::remove_file(test);
    let _ = std::fs::remove_file(out);
}
```

**Validation**: passes.

### Step 2.6 — Update PROBLEMS.md

**Action**: edit `doc/claude/PROBLEMS.md`:

- Mark P203 CLOSED.  Append fix-path: "phase 02 of plan 10
  (scope-exit gate loosened in `src/scopes.rs:1053`)."
- Update P204: "cleanup-side of P204 closed by plan 10 phase 02.
  Call-resolution side (tail-expression `n_inner(cell, args);
  return null_sentinel` shape) remains open."
- Move P203's table row from "open" to "closed" section if such
  a structure exists.

**Validation**: review.

## Acceptance for phase 02 overall

```bash
cargo run --bin loft --release -- tests/scripts/repro_p203.loft
echo "Exit: $?"   # Must be 0
test -f /tmp/repro_p203.txt    # Must exist

cargo test --release --test runtime_safety::p203_file_block_close_flushes_native
cargo test --release --test runtime_safety::p203_file_block_close_writes_to_disk
cargo test --release --test runtime_safety::scope_exit_does_not_depend_on_dep_being_empty

cargo test --release --test issues 2>&1 | tail -3            # 540/540
cargo test --release --test threading 2>&1 | tail -3
cargo test --release --test threading_chars 2>&1 | tail -3
cargo test --release --test native -- --test-threads=1 2>&1 | grep "native result"
```

## Commit shape

3-5 commits:
1. The 3-character gate edit (step 2.2) + regression tests (steps 2.4 + 2.5).
2. Per-regression suppression-list update (step 2.3) — one
   commit per case if multiple surface.
3. PROBLEMS.md update (step 2.6).

Ships as one PR.

## Problems encountered

_(append per problem — likely: phase 00 step 0.4 surfaced N
dep-tracking consumers; the loosening must not break their
expectations.  Document each as it surfaces)_

## Implementation notes

_(append per non-obvious decision — likely: how `set_skip_free`
propagates through the IR, whether it's per-var or per-Use)_
