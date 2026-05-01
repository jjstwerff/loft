# Phase 03 — Verify Drop semantics + add tests

**Status:** OPEN

**Kind:** Verification (the Drop safety net phase 02 relies on is
already in place per the survey; this phase pins it with tests
and documentation)

**Depends on:** Phase 00 step 0.2 confirmed Drop is already in
place.

**Optional**: phase 03 can be skipped if phase 02a confirms
phase 02 closed P203 cleanly with no fragility concerns.  Land
phase 03 only if defence-in-depth is wanted.

## What's already there (no code change needed)

`src/database/mod.rs:162` declares:

```rust
pub files: Vec<Option<std::fs::File>>,
```

When `OpFreeRef` runs `stores.files[file_ref] = None`
(`src/codegen_runtime.rs:117`), the previous `Some(File)` drops,
which closes the OS handle — that's how P203's fix actually
flushes: the *Drop* on the inner `std::fs::File` is what touches
the disk.

When `Stores` itself drops (program exit, panic unwind), `Vec`
drops every remaining `Some(File)` — every still-open file flushes
and closes.

What's **not** there but should be (added by this phase):
- Tests pinning Drop-on-replacement and Drop-on-Stores-drop
  invariants.
- Documentation in `LIFETIME.md` describing the contract.

## Detailed steps with validation

### Step 3.1 — Add Drop-on-replacement test

**Action**: append to `tests/runtime_safety.rs`:

```rust
#[test]
fn file_handle_drops_on_slot_replacement() {
    use std::io::Write;
    let path = "/tmp/p10_drop_replacement.txt";
    let _ = std::fs::remove_file(path);

    let mut files: Vec<Option<std::fs::File>> = Vec::new();
    files.push(Some(std::fs::File::create(path).unwrap()));
    if let Some(ref mut f) = files[0] {
        f.write_all(b"hello").unwrap();
    }

    // Replacing the slot with None drops the File → OS flush+close.
    files[0] = None;

    let content = std::fs::read_to_string(path).unwrap();
    assert_eq!(content, "hello",
        "file content should be flushed to disk after slot replacement");
    let _ = std::fs::remove_file(path);
}
```

**Validation**:
```bash
cargo test --release --test runtime_safety::file_handle_drops_on_slot_replacement
```

### Step 3.2 — Add Drop-on-Stores-drop test

**Action**:
```rust
#[test]
fn file_handles_drop_on_stores_drop() {
    use std::io::Write;
    use loft::database::Stores;

    let path = "/tmp/p10_drop_stores.txt";
    let _ = std::fs::remove_file(path);

    {
        let mut stores = Stores::new();
        stores.files.push(Some(std::fs::File::create(path).unwrap()));
        if let Some(ref mut f) = stores.files[0] {
            f.write_all(b"world").unwrap();
        }
        // Don't explicitly close.  Stores drops at end of scope.
    }

    let content = std::fs::read_to_string(path).unwrap();
    assert_eq!(content, "world",
        "file content should flush when Stores drops");
    let _ = std::fs::remove_file(path);
}
```

**Validation**:
```bash
cargo test --release --test runtime_safety::file_handles_drop_on_stores_drop
```

### Step 3.3 — Update LIFETIME.md

**Action**: extend the LIFETIME.md section added in phase 01:

```markdown
## File handle Drop contract

Plan 10 phase 02's scope-walk emits OpFreeRef at every block exit
for ref-shaped locals.  When the local refers to a file, OpFreeRef
sets `stores.files[i] = None`, which drops the underlying
`std::fs::File` — that drop is what flushes content to disk and
releases the OS handle.

Defence in depth: when `Stores` drops at program exit (or panic
unwind), `Vec<Option<File>>` drops every remaining `Some(File)`,
flushing and closing them.  Programs that never call OpFreeRef on
a file (because of a future emission gap) still get correct
cleanup at program exit.

The user-visible timing contract:
- **Block exit**: OpFreeRef → Drop fires → flush + close.
- **Function exit**: same path via outer scope's OpFreeRef.
- **Panic / abnormal exit**: Stores drops via stack unwind → all
  remaining files drop.  Best-effort flush; errors silently
  swallowed (file system may have partial content).

Regression tests: see `tests/runtime_safety.rs`.
```

**Validation**: review.

## Acceptance for phase 03 overall

```bash
cargo test --release --test runtime_safety::file_handle_drops_on_slot_replacement
cargo test --release --test runtime_safety::file_handles_drop_on_stores_drop
cargo test --release --test issues 2>&1 | tail -3            # 540/540
cargo test --release --test wrap file
```

No code change beyond test additions and doc.  The Drop semantics
are already correct.

## Commit shape

2 commits (tests + docs); ships as one PR.

## Problems encountered

_(append per problem — likely: discovering that some File use
sites don't go through stores.files (e.g., the `Read` side has
its own representation), in which case Drop coverage is partial
and needs unification)_

## Implementation notes

_(append per non-obvious decision — likely: should OpFreeRef
also set the slot to None defensively when ref_count > 0?
Currently it only frees on last ref.  If multiple refs to the
same file exist — possible? — Drop happens too late)_
