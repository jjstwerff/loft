# Phase 01 — Cell/Stores ABI consolidation

**Status:** OPEN

**Kind:** Simplification (no P-issue closes)

**Depends on:** Phase 00.

## What's tangled today

P199.A migrated most runtime fns from `&mut Stores` to
`&UnsafeCell<Stores>` (the "cell" ABI), but a few helpers still
take `stores`.  The codegen tracks which ABI to use via a
**hardcoded name list** in two places:

1. `src/generation/calls.rs:output_call_user_fn` — checks
   `LEGACY_STORES_FNS` to pick `stores` vs `cell`.
2. `src/generation/dispatch.rs:163-228` (the arg-hoist path) —
   duplicates the same hardcoded list:

```rust
const LEGACY_STORES_FNS: &[&str] = &[
    "n_now", "n_ticks", "n_get_store_lock", "n_set_store_lock",
    "n_rand", "n_rand_indices", "n_parallel_for_native",
    "n_parallel_for_ref_native", "n_path_sep", "n_stack_trace",
    "n_hash_sorted",
];
```

Adding a new runtime fn means remembering to update both lists.
Renaming a fn means a silent miscompile if either list isn't
updated.  Canonical name-based-switching anti-pattern.

## Detailed steps with validation

### Step 1.1 — Introduce `Abi` enum + `RuntimeFn` registry struct

**Action**: in `src/codegen_runtime.rs` add:
```rust
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Abi { Cell, LegacyStores }

pub struct RuntimeFn {
    pub name: &'static str,
    pub abi: Abi,
}
```

Replace existing `CODEGEN_RUNTIME_FNS: &[&str]` (or whatever shape
it has today) with `&[RuntimeFn]`.  Tag each entry: anything in the
old `LEGACY_STORES_FNS` list → `Abi::LegacyStores`; everything else
→ `Abi::Cell`.

Add `pub fn abi_of(name: &str) -> Abi`.

**Validation**:
```bash
cargo build --release
cargo test --release --test issues 2>&1 | tail -3            # 540/540 (no behaviour change yet)
cargo test --release --test codegen_emitter                  # baseline diff still byte-identical
```

### Step 1.2 — Replace `LEGACY_STORES_FNS` in `calls.rs` with `abi_of`

**Action**: in `output_call_user_fn`, replace the contains-check
against `LEGACY_STORES_FNS` with `abi_of(name) == Abi::LegacyStores`.
Delete the local constant.

**Validation**: byte-identical baseline diff still passes (the
`abi_of` lookup must produce the same bool result).

```bash
cargo test --release --test codegen_emitter::baseline_emission_unchanged
```

### Step 1.3 — Replace `LEGACY_STORES_FNS` in `dispatch.rs` with `abi_of`

**Action**: same edit, in the arg-hoist path.  Delete the duplicate
constant.

**Validation**: same diff test.  Search the codebase to confirm no
remaining references:
```bash
rg LEGACY_STORES_FNS src/   # expect: empty
```

### Step 1.4 — Add programmatic ABI consistency test

**Action**: in `tests/codegen_emitter.rs`:
```rust
#[test]
fn no_hardcoded_abi_lists_remain() {
    let src = std::fs::read_to_string("src/generation/calls.rs").unwrap();
    assert!(!src.contains("LEGACY_STORES_FNS"),
        "calls.rs still has hardcoded ABI list");
    let src = std::fs::read_to_string("src/generation/dispatch.rs").unwrap();
    assert!(!src.contains("LEGACY_STORES_FNS"),
        "dispatch.rs still has hardcoded ABI list");
}

#[test]
fn abi_of_handles_all_runtime_fns() {
    use loft::codegen_runtime::{CODEGEN_RUNTIME_FNS, abi_of, Abi};
    for fn_def in CODEGEN_RUNTIME_FNS {
        assert_eq!(abi_of(fn_def.name), fn_def.abi,
            "abi_of disagrees with registry for {}", fn_def.name);
    }
    // Unknown name → Cell (user-fn default)
    assert_eq!(abi_of("nonexistent_fn"), Abi::Cell);
}
```

**Validation**:
```bash
cargo test --release --test codegen_emitter::no_hardcoded_abi_lists_remain
cargo test --release --test codegen_emitter::abi_of_handles_all_runtime_fns
```

### Step 1.5 — Migrate one legacy fn to cell ABI (e.g., `n_now`)

**Action**: rewrite `n_now` in `codegen_runtime.rs`:
```rust
pub fn n_now(cell: &std::cell::UnsafeCell<Stores>) -> i64 {
    let _stores: &mut Stores = unsafe { &mut *cell.get() };
    // existing body, using _stores if needed
}
```

Flip its tag to `Abi::Cell`.  The call sites in generated code
automatically use `cell` instead of `stores`.

**Validation**:
```bash
cargo test --release --test issues 2>&1 | tail -3            # 540/540
cargo test --release --test wrap time                        # n_now is the time fn
cargo test --release --test native -- --test-threads=1 2>&1 | grep "native result"

# Diff baseline emission for a test that uses n_now (e.g., a time test):
cargo run --bin loft --release --quiet -- --native-emit /tmp/p09-step15.rs tests/docs/22-time.loft
# Expect: the n_now call site now passes `cell`, not `stores`.
grep "n_now(" /tmp/p09-step15.rs
# Expected output: lines containing `n_now(cell` not `n_now(stores`.
```

### Step 1.6 — Migrate remaining legacy fns

**Action**: repeat step 1.5 for each remaining `Abi::LegacyStores`
entry, one commit per fn:
- `n_ticks`, `n_get_store_lock`, `n_set_store_lock`, `n_rand`,
  `n_rand_indices`, `n_path_sep`, `n_stack_trace`, `n_hash_sorted`
- `n_parallel_for_native`, `n_parallel_for_ref_native` — these
  coordinate with phase 03; defer until phase 03 lands.

**Validation per migration**: same gate as step 1.5, plus the
specific wrap suite for the migrated fn (e.g., `wrap rng` for
`n_rand`).

### Step 1.7 — Cleanup when `Abi::LegacyStores` count == 0

**Action**: when no entry tags `LegacyStores` anymore (likely after
phase 03 migrates the parallel fns):
- Delete the `Abi` enum.
- Simplify `abi_of` to return unit or remove it.
- Remove the `if/else` ABI selection in `output_call_user_fn` and
  the arg-hoist path — they always emit `cell`.

**Validation**:
```bash
rg "Abi::LegacyStores" src/   # expect: empty
cargo test --release --test issues 2>&1 | tail -3            # 540/540
cargo test --release --test codegen_emitter::baseline_emission_unchanged
```

## Acceptance for phase 01 overall

```bash
cargo test --release --test codegen_emitter
cargo test --release --test issues 2>&1 | tail -3
cargo test --release --test threading 2>&1 | tail -3
cargo test --release --test native -- --test-threads=1 2>&1 | grep "native result"
```

Plus: `rg LEGACY_STORES_FNS src/` returns nothing.

## Commit shape

10-12 commits across the steps; ships as one PR.

## Problems encountered

_(append per problem — likely: `n_parallel_for_native` migration
requires updating the closure-shape emitter in `dispatch.rs` —
overlap with phase 03)_

## Implementation notes

_(append per non-obvious decision)_
