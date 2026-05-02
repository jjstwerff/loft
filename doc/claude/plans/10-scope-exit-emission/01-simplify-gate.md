# Phase 01 — Simplify the gate

**Status:** OPEN — deferred.

**Kind:** Simplification (no P-issue closure; pure cognitive
clarity win).

**Depends on:** Phase 00 (characterisation) re-confirmed against
the current code state when work starts.

## Goal

Drop the `(dep.is_empty() || is_work_ref) &&` prefix from the
cleanup gate at `src/scopes.rs:1053`, leaving:

```rust
let emit = !in_ret && !function.is_skip_free(v);
```

After: cleanup correctness no longer depends on dep-tracking
precision.  Dep-tracking stays for non-cleanup purposes (aliasing
analysis, closure capture, parallel isolation).

## Why this is safe

Per phase 00, the runtime invariants that make this safe are
already in place:

1. **OpFreeRef no-ops on already-freed slots** — extra calls cost
   nanoseconds, not correctness.
2. **`Vec<Option<File>>` drops on slot replacement** — file
   handles flush + close correctly when the slot clears.
3. **Reference counting protects shared records** — multi-ref
   cases don't double-close.

The current gate is an optimisation that masks emission gaps when
the dep tracker mishandles a var.  The optimisation isn't worth
the brittleness.

## Detailed steps with validation

### Step 1.1 — Re-confirm phase 00 characterisation

**Action**: read `src/scopes.rs:1001-1112` and verify the gate
condition is still as documented in 00-characterize.md.  If it has
drifted, update phase 00's characterisation before proceeding.

**Validation**: visual diff against phase 00 doc.

### Step 1.2 — Survey current suppression usage

**Action**: refresh the phase 00 suppression catalogue with a
current `rg`:

```bash
rg -n "skip_free|set_skip_free|mark_skip_free" \
    src/parser/ src/scopes.rs src/variables/ \
    --type rust > /tmp/p10-skip-free.txt
```

For each entry, classify whether the suppression is:
- (a) Already correct — the var has owned-by-callee semantics and
  shouldn't be freed at scope exit.
- (b) Compensating for a dep-tracker gap — the var SHOULD be
  freed but the gate would emit a duplicate OpFreeRef without
  the suppression.

Class (a) entries stay.  Class (b) entries can be removed once
the gate is loosened (the runtime no-op handles the duplicate
cases).

**Validation**: written classification of every suppression
site.

### Step 1.3 — Apply the gate edit

**Action**: in `src/scopes.rs:1053`, change:

```rust
let emit = (dep.is_empty() || is_work_ref) && !in_ret && !function.is_skip_free(v);
```

to:

```rust
// Plan 10 phase 01: scope-walk emits OpFreeRef for any ref-shaped
// local at scope exit unless the var participates in the return
// path or is explicitly marked skip_free.  The runtime no-ops on
// already-freed slots (see src/codegen_runtime.rs:100), so the
// dep gate is no longer load-bearing for correctness — it was an
// optimisation that masked emission gaps.
let emit = !in_ret && !function.is_skip_free(v);
```

**Validation**:

```bash
cargo build --release
cargo test --release --test issues 2>&1 | tail -3        # 540/540
cargo test --release --test threading 2>&1 | tail -3
cargo test --release --test threading_chars 2>&1 | tail -3
cargo test --release --test native -- --test-threads=1 2>&1 | grep "native result"
```

If anything regresses, classify the failing test:
- Test relied on OpFreeRef NOT firing for a specific var → mark
  the var with `set_skip_free` (extension of the explicit-suppression
  mechanism).
- Test exposes a real bug the old gate masked → file separately
  before proceeding.

### Step 1.4 — Audit and update suppression list

**Action**: for each regression from step 1.3, add an explicit
`is_skip_free` marker at the appropriate point in the parser.
Update phase 00's catalogue with each new entry and its
rationale.

**Validation**: full suite green; no regressions; suppression
list documented.

### Step 1.5 — Add structural test

**Action**: pin the new contract:

```rust
// tests/scopes_simplification.rs (new)
#[test]
fn cleanup_emission_does_not_depend_on_dep_state() {
    // Compile a small program where the file ref has a non-empty dep
    // (originates from a function call returning a ref).  The
    // generated code must contain OpFreeRef for that local at scope
    // exit, regardless of dep-tracker analysis.
    let test = "tests/scripts/p10_simpl_dep_nonempty.loft";
    std::fs::write(test, r#"
fn make_file(p: text) -> file { file(p) }
fn main() {
    {f = make_file("/tmp/p10_simpl.txt"); f += "x"}
}
    "#).unwrap();

    let out = "/tmp/p10_simpl.rs";
    let status = std::process::Command::new("cargo")
        .args(["run", "--bin", "loft", "--release", "--quiet", "--",
               "--native-emit", out, test])
        .status().unwrap();
    assert!(status.success());

    let src = std::fs::read_to_string(out).unwrap();
    assert!(src.contains("OpFreeRef"),
        "phase 01 contract: scope-walk emits OpFreeRef regardless of dep state");
    let _ = std::fs::remove_file(test);
    let _ = std::fs::remove_file(out);
}
```

**Validation**:

```bash
cargo test --release --test scopes_simplification
```

### Step 1.6 — Update LIFETIME.md

**Action**: in `doc/claude/LIFETIME.md`, document the new
contract:

> **Scope-exit cleanup contract (post plan 10 phase 01):**
> OpFreeRef fires at scope exit for every ref-shaped local that
> isn't `is_skip_free` or part of the return path.  The dep
> tracker no longer gates cleanup emission; it stays for aliasing
> analysis, closure capture, and parallel isolation.  OpFreeRef
> safety on already-freed slots makes the conservative emission
> safe at runtime.

**Validation**: review.

## Acceptance for phase 01 overall

```bash
cargo test --release --test scopes_simplification
cargo test --release --test issues 2>&1 | tail -3
cargo test --release --test threading 2>&1 | tail -3
cargo test --release --test threading_chars 2>&1 | tail -3
cargo test --release --test native -- --test-threads=1 2>&1 | grep "native result"
```

## Commit shape

3-5 commits across the steps; ships as one PR.

## Out of scope

- Any P-issue closure.  None expected; if one happens, it's a
  bonus.
- Removing dep-tracking entirely.  Only the cleanup-emission
  half is touched.
- Any change to `src/codegen_runtime.rs`.  The runtime is already
  correct.

## Problems encountered

_(append per problem when phase 01 starts)_

## Implementation notes

_(append per non-obvious decision)_
