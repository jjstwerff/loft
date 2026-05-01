# Phase 04 — Key-keyed Op emitter

**Status:** OPEN

**Kind:** Simplification — independent of bug-fix phases.  Lands
when convenient; not on the critical path for P-issue closures.
Reduces duplication in `OpGetRecord` / `OpIterate` shared
key-decode logic.

**Depends on:** Phase 00.

## What's tangled today

`OpGetRecord` and `OpIterate` both take a variable-length key list
and must emit `Content::Type(…)` wrappers based on per-key type
codes (`-7..=+7`).  `dispatch.rs:708-794` handles them as two
separate special cases that share most of their logic:

- `OpGetRecord` (708-734): lookup typed-store schema by `db_tp`,
  walk key positions, emit one `Content::*` per key argument.
- `OpIterate` (736-794): same key decoding, but `from`/`till`
  arrays produce two parallel `Content` lists; key types come from
  a `Value::Keys` payload, not the schema.

## Detailed steps with validation

### Step 4.1 — Capture key-Op emission corpus

**Action**: capture goldens for representative tests:
- `tests/scripts/sorted_basic.loft` — uses `OpGetRecord`
- `tests/scripts/sorted_iter.loft` — uses `OpIterate` with from+till
- `tests/scripts/index_lookup.loft` — different schema shape

```bash
mkdir -p tests/golden/key_ops
for t in tests/scripts/sorted_basic.loft tests/scripts/sorted_iter.loft \
         tests/scripts/index_lookup.loft; do
    name=$(basename "$t" .loft)
    cargo run --bin loft --release --quiet -- \
        --native-emit "tests/golden/key_ops/$name.rs" "$t"
done
```

If listed tests don't exist, write minimal reproducers first.

**Validation**: each golden compiles + runs.

### Step 4.2 — Extract `emit_content_array` + parse helpers

**Action**: create `src/generation/ops/key_ops.rs` with shared
helpers:
- `emit_content_array(ctx, vals, key_types)`
- `parse_get_record_args(args) -> (Value, i32, &[Value])`
- `parse_iterate_args(args) -> IterateParsed { keys, from, till }`
- `emit_keys_array(ctx, keys)`

Add unit tests:
```rust
#[test]
fn emit_content_array_handles_mixed_types() {
    let mut buf = Vec::new();
    let vals = vec![Value::Int(42), Value::Text("hi".into()), Value::Null];
    let key_types = vec![1, 4, 1];
    emit_content_array(&mut ctx_with(&mut buf), &vals, &key_types).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(s.starts_with("&["));
    assert!(s.contains("Content::"));
}

#[test]
fn parse_iterate_args_splits_from_and_till() {
    // [data, on, arg, Keys([...]), 2, from0, from1, 2, till0, till1]
    let args = vec![
        Value::Var(0), Value::Int(0), Value::Int(0),
        Value::Keys(vec![/* … */]),
        Value::Int(2), Value::Int(10), Value::Int(20),
        Value::Int(2), Value::Int(50), Value::Int(60),
    ];
    let parsed = parse_iterate_args(&args).unwrap();
    assert_eq!(parsed.from.len(), 2);
    assert_eq!(parsed.till.len(), 2);
}
```

**Validation**:
```bash
cargo test --release --test codegen_emitter::emit_content_array_handles_mixed_types
cargo test --release --test codegen_emitter::parse_iterate_args_splits_from_and_till
```

### Step 4.3 — Implement and register `OpGetRecordEmitter`

**Action**: implement the emitter using the helpers; register it.
Replace the `OpGetRecord` block in `dispatch.rs:708-734` with
`return emit_op(ctx, "OpGetRecord", args)`.

**Validation**:
```bash
diff tests/golden/key_ops/sorted_basic.rs <(cargo run --bin loft --release --quiet -- --native-emit /dev/stdout tests/scripts/sorted_basic.loft)
diff tests/golden/key_ops/index_lookup.rs <(...)
cargo test --release --test wrap sorted
cargo test --release --test wrap index
```

### Step 4.4 — Implement and register `OpIterateEmitter`

**Action**: same shape, for `OpIterate`.  Replace
`dispatch.rs:736-794` with the emitter dispatch.

**Validation**:
```bash
diff tests/golden/key_ops/sorted_iter.rs <(...)
cargo test --release --test wrap sorted
cargo test --release --test issues 2>&1 | tail -3
```

### Step 4.5 — Structural test

**Action**:
```rust
#[test]
fn no_key_op_special_case_in_dispatch() {
    let src = std::fs::read_to_string("src/generation/dispatch.rs").unwrap();
    assert!(!src.contains(r#""OpGetRecord" =>"#),
        "dispatch.rs still has OpGetRecord special case");
    assert!(!src.contains(r#""OpIterate" =>"#),
        "dispatch.rs still has OpIterate special case");
}
```

**Validation**: test passes.

## Acceptance for phase 04 overall

```bash
cargo test --release --test codegen_emitter::no_key_op_special_case_in_dispatch
cargo test --release --test wrap sorted
cargo test --release --test wrap index
cargo test --release --test issues 2>&1 | tail -3
cargo test --release --test native -- --test-threads=1 2>&1 | grep "native result"
```

Plus all `tests/golden/key_ops/*.rs` match emission exactly.

Net diff target: ~85 lines deleted from `dispatch.rs`, ~110 lines
added across `src/generation/ops/key_ops.rs`.

## Commit shape

5-6 commits across the steps; ships as one PR.

## Problems encountered

_(append per problem)_

## Implementation notes

_(append per non-obvious decision)_
