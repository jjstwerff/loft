# Phase 01 — Verify runtime safety + add tests

**Status:** OPEN

**Kind:** Verification (the runtime safety phase 02 relies on is
already in place per the survey; this phase pins it with tests)

**Depends on:** Phase 00 step 0.1 confirmed the runtime is already
safe.

## What's already there (no code change needed)

`src/codegen_runtime.rs:98-122` already has:

```rust
pub fn OpFreeRef(cell: &std::cell::UnsafeCell<Stores>, db: DbRef, name: &str) {
    let stores: &mut Stores = unsafe { &mut *cell.get() };
    if db.store_nr == u16::MAX {
        return;                      // already-freed / null sentinel
    }
    if (db.store_nr as usize) >= stores.allocations.len() {
        return;                      // out-of-bounds store_nr
    }
    // …file handle close logic + free_named…
}
```

These two early returns make OpFreeRef safe for the cases phase 02
will trigger more often:
- Vars whose slot was freed by a prior OpFreeRef (double-free
  protection).
- Null DbRef sentinels (deliberate "no value here").
- Wrong-store-nr from a corrupted DbRef (silently no-ops).

What's **not** there but should be (added by this phase):
- Tests pinning the safety invariant (so future refactors can't
  regress it silently).
- Updated documentation in `LIFETIME.md` describing the contract.

## Detailed steps with validation

### Step 1.1 — Add a regression test for already-freed safety

**Action**: append to `tests/codegen_emitter.rs` (or create the
file if it doesn't exist yet from plan 09):

```rust
// tests/runtime_safety.rs (new)
use loft::codegen_runtime::OpFreeRef;
use loft::database::Stores;
use loft::keys::DbRef;

#[test]
fn op_free_ref_noop_on_null_sentinel() {
    let cell = std::cell::UnsafeCell::new(Stores::new());
    let null = DbRef { store_nr: u16::MAX, rec: 0, pos: 8 };
    OpFreeRef(&cell, null, "test_null_sentinel");  // must not panic
}

#[test]
fn op_free_ref_noop_on_out_of_bounds_store_nr() {
    let cell = std::cell::UnsafeCell::new(Stores::new());
    let bogus = DbRef { store_nr: 12345, rec: 0, pos: 0 };
    OpFreeRef(&cell, bogus, "test_oob");  // must not panic
}

#[test]
fn op_free_ref_noop_on_double_free() {
    let cell = std::cell::UnsafeCell::new(Stores::new());
    // Allocate a real ref, free it, free it again.
    let stores = unsafe { &mut *cell.get() };
    let db = stores.create_record(/* … */);  // shape per actual API
    let db_copy = db;
    OpFreeRef(&cell, db, "test_double_a");      // real free
    OpFreeRef(&cell, db_copy, "test_double_b"); // must not panic
}
```

Adjust `create_record(…)` to match the actual Stores API; the test
just needs a real ref to free twice.

**Validation**:
```bash
cargo test --release --test runtime_safety
# Expected: 3 tests, all pass.
```

### Step 1.2 — Add a regression test for file-handle close on free

**Action**:
```rust
// tests/runtime_safety.rs
#[test]
fn op_free_ref_closes_file_handle_on_last_ref() {
    let path = "/tmp/p10_test_close.txt";
    let _ = std::fs::remove_file(path);
    let cell = std::cell::UnsafeCell::new(Stores::new());

    // Manually allocate a file handle in stores.files and a ref
    // pointing to it, then call OpFreeRef.
    // (Use the same machinery as `n_file` would.)

    let stores = unsafe { &mut *cell.get() };
    // … set up: stores.files.push(Some(File::create(path)));
    //          allocate a ref to a file struct that has the file_ref
    //          slot pointing at index 0 in stores.files
    let file_db = /* …allocate via the same path n_file uses… */;

    // Write something through the handle
    use std::io::Write;
    if let Some(ref mut f) = stores.files[0] {
        f.write_all(b"hello").unwrap();
    }

    OpFreeRef(&cell, file_db, "test_file");

    // Drop closed the OS handle; content should now be on disk.
    let content = std::fs::read_to_string(path).unwrap();
    assert_eq!(content, "hello");
    assert!(unsafe { &*cell.get() }.files[0].is_none(),
        "file slot should be None after OpFreeRef");
}
```

Exact setup depends on the `n_file` machinery; refine during
implementation.

**Validation**:
```bash
cargo test --release --test runtime_safety::op_free_ref_closes_file_handle_on_last_ref
```

### Step 1.3 — Update LIFETIME.md

**Action**: in `doc/claude/LIFETIME.md`, add a section documenting
the OpFreeRef safety contract:

```markdown
## OpFreeRef runtime safety contract

`src/codegen_runtime.rs::OpFreeRef` is safe to call on:

- A live ref: reclaims the slot, decrements ref-count, closes any
  associated file handle when ref-count drops to zero.
- An already-freed ref (`store_nr == u16::MAX`): silent no-op.
- A null sentinel (also `store_nr == u16::MAX`): silent no-op.
- An out-of-bounds store_nr: silent no-op (defensive).

Plan 10's scope-walk (phase 02) relies on this safety contract:
the parser may emit OpFreeRef for vars whose slot has been freed
by a prior emission or whose value is a null sentinel.  The
runtime absorbs those silently.

File handles: when OpFreeRef closes a file ref's last live use,
`stores.files[file_ref] = None` triggers Drop on the underlying
`std::fs::File`, which closes the OS handle.

Regression tests: see `tests/runtime_safety.rs`.
```

**Validation**: review.

## Acceptance for phase 01 overall

```bash
cargo test --release --test runtime_safety
cargo test --release --test issues 2>&1 | tail -3            # 540/540
cargo test --release --test threading 2>&1 | tail -3
cargo test --release --test threading_chars 2>&1 | tail -3
cargo test --release --test native -- --test-threads=1 2>&1 | grep "native result"
```

No behaviour change.  The test file is the only new code.

## Commit shape

2-3 commits across the steps; ships as one PR.

## Problems encountered

_(append per problem)_

## Implementation notes

_(append per non-obvious decision — likely: the file-handle test
needs careful setup of the Stores; if the `n_file` path is too
gnarly to reproduce in a unit test, use an integration test that
runs a small loft program instead)_
