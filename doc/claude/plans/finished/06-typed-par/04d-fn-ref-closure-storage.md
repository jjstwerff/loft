<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan 06 phase 4d.C — Fn-ref struct field stores closure (always)

## Context

After phase 4d.A (typed worker-input dispatch) and 4d.B (sorted/hash/index
materialise), `Type::Function` got a working struct-field path that
stores **only the 4-byte `i32` d_nr** and discards the 12-byte closure
DbRef.  This was a deliberate simplification — bare named-function
fn-refs (`f: dbl`) carry a null closure already, so dropping the
trailing 12 bytes was lossless for the common case.

The simplification has two pain points:

1. **Lambdas with captures truncate silently.**  Putting a capturing
   lambda into a struct field discards its captured state.  No
   diagnostic — the lambda runs on `null` closure at call time.
2. **Native codegen has a width-mismatch hack.**  The native fn-ref
   representation is `(u32, DbRef)`, the storage representation is 4
   bytes, so the parser has to special-case fn-ref-shaped values
   (P196: `var_tmp.0 as i32` rejects on a tuple).  Today's tree
   leaves the parser emitting `OpSetFnRef` calls for an opcode that
   doesn't exist, breaking native compilation for any non-literal
   fn-ref source.

The user's call: **closures are the common real-world case.  Make
storage hold the full fn-ref (d_nr + closure DbRef) instead of the
truncated form.**  This document is the design.

## Goal

Store every fn-ref struct field (and tuple-element fn-ref) as a
**16-byte slot containing both halves**: `4B i32 d_nr + 12B closure
DbRef`.  Native and stack representations already match the storage
shape (`(u32, DbRef)` natively, 8B-i64 + 12B-DbRef on the
interpreter stack — same total).  Reads and writes round-trip the
closure intact; null closures still cost 12 zero bytes (acceptable).

## Design

### 1. Storage layout

| Slot | Bytes | Content |
|---|---|---|
| 0..3 | 4 | `i32` d_nr (the function's def-nr — `i32::MIN` is the null sentinel) |
| 4..15 | 12 | `DbRef` closure (`store_nr: u16` padded to 4 + `rec: u32` + `pos: u32`) |
| **total** | **16** | |

The closure DbRef's `store_nr == u16::MAX` sentinel is preserved as
"no closure".

**Same-store rule.**  When a fn-ref is assigned to a struct field
whose host record is in store *S*, the closure record must already
live in *S*.  Cross-store assignment deep-copies the closure record
into *S* (mirrors how text fields intern strings into the host's
store).  Loft's existing dependency-tracking machinery
(`Type::Reference(_, deps)`) supports the lifetime check; the actual
copy is via `OpCopyRecord` over the closure's record type.

### 2. Synthetic `__fn_ref` struct

Mirror the existing `__tuple<…>` infrastructure.  At first reference
to `Type::Function`, register a global synthetic struct named
`__fn_ref<args→ret>` with:

- attribute `_d_nr`: `Type::Integer(forced_size: 4)` — 4 bytes.
- attribute `_closure`: a 12-byte storage cell holding a DbRef.

`fill_database` resolves `Type::Function` → `__fn_ref`'s
`known_type`.  Layout via `calculate_positions_with_groups` (the
shape is two fields, no LinkedFieldGroup needed unless tuple-element
groups demand co-location).

### 3. New opcodes

Two new opcodes.  Both follow the existing `OpGet*` / `OpSet*`
declaration pattern in `default/01_code.loft` — short `#rust"…"`
bodies, dispatch table entry in `src/fill.rs`.

#### OpSetDbRef(v1: reference, fld: const u16, val: reference)

Writes 12 bytes of `DbRef` (store_nr + rec + pos) at the field
offset.  Native body:

```rust
{{let db = @v1;
  let v = @val;
  let s = stores.store_mut(&db);
  s.set_u32_raw(db.rec, db.pos + u32::from(@fld), u32::from(v.store_nr));
  s.set_u32_raw(db.rec, db.pos + u32::from(@fld) + 4, v.rec);
  s.set_u32_raw(db.rec, db.pos + u32::from(@fld) + 8, v.pos);
}}
```

Interpreter (`src/fill.rs`): pop 12B `DbRef`, pop 12B host ref, write
3 × 4-byte words.

#### OpGetDbRef(v1: reference, fld: const u16) -> reference

Reads 12 bytes, returns `DbRef`.  Native body symmetrical.

The two new opcodes serve **any** stored 12-byte DbRef payload —
not just fn-ref closures.  They become the natural primitive for
storing references-by-pointer (currently only `Type::Vector` /
`Hash` / etc. have this shape, via `OpSetInt4` on a 4-byte rec
pointer with implicit same-store).

### 4. Set codegen

`src/parser/mod.rs::set_field_check::Type::Function`:

```rust
Type::Function(_, _, _) => {
    // Always 16 bytes: 4B d_nr at off+0, 12B closure DbRef at off+4.
    // val_code's IR shape determines how we extract each half.
    let (d_nr_part, closure_part) = split_fn_ref_value(val_code);
    let pos_d_nr = pos_val.clone();
    let pos_closure = Value::Int(pos_val.as_int() + 4);
    Value::Block([
        cl("OpSetInt4", &[ref_code.clone(), pos_d_nr, d_nr_part]),
        cl("OpSetDbRef", &[ref_code, pos_closure, closure_part]),
    ])
}
```

`split_fn_ref_value(val)` produces the two IR halves:

- `Value::Int(d_nr)` → `(Value::Int(d_nr), Value::Null_DbRef)` —
  bare named function, null closure.
- `Value::FnRef(d_nr, clos_var, _)` → `(Value::Int(d_nr),
  Value::Var(clos_var))` — lambda with explicit closure variable.
- `Value::Var(v)` (v of `Type::Function`) → `(<v.0>, <v.1>)` via
  TupleGet-equivalent projections.  Native sees `var_v.0` /
  `var_v.1`; interpreter reads the d_nr's 8B + closure's 12B from v's
  20-byte slot.
- Anything else (function call return, etc.) → stash to a temp
  fn-ref var, then project.

Mirrors the existing tuple-element split path (`emit_tuple_set_ops`
→ `emit_set_one_element`).

### 5. Get codegen

`src/parser/mod.rs::get_val::Type::Function`:

```rust
Type::Function(_, _, _) => {
    // Read 4B d_nr and 12B closure DbRef; assemble into 20-byte
    // stack fn-ref slot via Block.
    let read_dnr = cl("OpGetInt4", &[code.clone(), p.clone()]);
    let read_clos = cl("OpGetDbRef", &[code, Value::Int(p.as_int() + 4)]);
    v_block(vec![read_dnr, read_clos], tp.clone(), "fn_ref_field_read")
}
```

Same Block trick the existing `fn_ref_field_read` uses.  Replaces
the `OpNullRefSentinel` 12-byte filler with a real `OpGetDbRef`.
Native emit's special-case for the `fn_ref_field_read` block name
already handles the `(u32, DbRef)` tuple assembly — extend it to
read both halves dynamically instead of synthesising a null
sentinel for the second.

### 6. Tuple-element fn-ref

`src/parser/mod.rs::emit_set_one_element::Type::Function`: same
two-write path — `OpSetInt4` at the tuple's `pos+0` for d_nr,
`OpSetDbRef` at `pos+4` for the closure.  The synthetic `__tuple<…>`
struct's element layout for a `Type::Function` element becomes 16
bytes (matching the `__fn_ref` synthetic).

`get_val::Type::Tuple`'s recursion into `Type::Function` element:
no change — it already calls `get_val(Type::Function, …)`, which
the new arm handles.

### 7. Same-store closure-copy

When `set_field_check::Type::Function` runs for a `val_code` whose
closure DbRef points at a **different store** than the host record,
the codegen inserts an `OpCopyRecord` to deep-copy the closure
record into the host's store, then writes the new local DbRef.

Detection: closure's source store_nr ≠ host's store_nr.  At parse
time we can't always know (the host is computed at runtime), so the
check is runtime-side.  Add a runtime helper `OpCopyClosureIntoHost`
that takes the host DbRef + the closure DbRef, copies the closure
record if needed, returns the (possibly-rewritten) closure DbRef.
The set arm uses this output as `closure_part`.

### 8. Native template handling

The existing `(u32, DbRef)` Rust representation already matches the
new 16-byte storage layout byte-for-byte:

- Bytes 0..3: u32 (d_nr).
- Bytes 4..7: u32 padding to align the inner struct.
- Bytes 8..15: u32 rec + u32 pos (no store_nr in the storage —
  closures are same-store).

Wait — that doesn't match.  The native rep is `(u32, DbRef)` =
`(u32, (u16, u32, u32))` whose Rust layout (with default repr) puts
DbRef at offset 4 with a 4-byte alignment, total 16 bytes (DbRef is
already 12 with `u16` padded to 4).  The 12 bytes are
store_nr+padding+rec+pos.

For storage, we want byte-for-byte match so reads/writes can use
plain `set_u32_raw` triples.  Two options:

- **(a) Store full DbRef** (12 bytes including store_nr): the
  native rep matches storage exactly.  Simpler.  Cost: 4 wasted
  bytes per fn-ref field for the redundant store_nr (since closures
  are always same-store, store_nr is implied).
- **(b) Store only rec+pos** (8 bytes): native reads reconstruct
  store_nr from the host's store_nr at load time.  Saves 4 bytes
  per field but needs an explicit reconstruct step in get
  codegen.

Recommend **(a)** for symmetry and simplicity.  The 4-byte overhead
is negligible at the field-count scales loft programs hit (single
digits to low hundreds of fn-ref fields per record max).

### 9. Removes today's hacks

- Parser's `OpSetFnRef` calls (lines 1918, 2120 in
  `src/parser/mod.rs` after the partial revert) — **deleted**.
- Native template's `val_is_char`-style fn-ref projection — **not
  added** (the native rep matches storage; no special projection
  needed).
- The "literal d_nr → Value::Int reduction" in the Type::Function
  arm — **kept** for the bare-named-function fast path; the closure
  half just becomes the null sentinel.
- `OpNullRefSentinel` filler in `fn_ref_field_read` block — **kept
  for backward compat** but unused once the new arm lands.

### 10. Test coverage

| Case | Test name | Today | After 4d.C |
|---|---|---|---|
| Bare named fn in struct field, write + read + call | `p4d_fn_ref_as_struct_field` | passing | passing |
| Default-init `Holder {}` | `p4d_fn_ref_field_default_init` | passing (P193) | passing |
| Tuple-of-fn-ref, literal source | already in `p4d_*` | passing | passing |
| Tuple-of-fn-ref, **non-literal** source (function returns tuple) | new: `p4d_fn_ref_field_via_call_return` | **fails** (P196) | passing |
| Capturing lambda assigned to struct field, then called | new: `p4d_fn_ref_field_lambda_with_capture` | silently truncates closure | passing |
| Capturing lambda in tuple field | new: `p4d_tuple_field_lambda_with_capture` | silently truncates | passing |
| Cross-store closure copy (struct in store A, lambda captured from store B) | new: `p4d_fn_ref_field_cross_store_closure` | **fails / UB** | passing |

## Status (2026-04-28)

- **Step 1 — Synthetic `__fn_ref` struct registration**: ✅ shipped.
  `src/data.rs::Data::fn_ref_def` registers a global struct with
  `_d_nr` (i32 size 4) + placeholder `_closure` attribute; `type_def_nr`
  + `type_elm` route `Type::Function` to it.
- **Step 2 — `Parts::DbRef` 12-byte raw DbRef storage shape + new
  opcodes**: ✅ shipped.
  - `Parts::DbRef` variant added in `src/database/mod.rs`; arms wired
    through `database/io.rs`, `database/structures.rs`,
    `database/format.rs`, `database/search.rs` (panic for non-collection
    operations, debug-format renders as `DbRef(s,r,p)` or `null`).
  - `Stores::dbref()` registers a 12-byte primitive type with
    `Parts::DbRef`.
  - `OpSetDbRef(v1, fld, val)` and `OpGetDbRef(v1, fld) -> reference`
    declared in `default/01_code.loft`; OPERATORS array grown 243 → 245;
    interpreter dispatch in `src/fill.rs` wired (`set_db_ref` /
    `get_db_ref` write/read 3 × u32 raw words).
- Steps 3–10: still pending.

## Effort

| Step | Files | Effort |
|---|---|---|
| 1. Synthetic `__fn_ref` struct registration | `src/data.rs` (new `fn_ref_def` mirroring `tuple_def`) | XS |
| 2. Two new opcodes (OpSetDbRef / OpGetDbRef) | `default/01_code.loft`, `src/fill.rs` (declaration + dispatch table grow to 245) | S |
| 3. `fill_database::Type::Function` routes to synthetic | `src/typedef.rs` (existing arm retargeted) | XS |
| 4. Set codegen — split + 2 writes | `src/parser/mod.rs::set_field_check`, `emit_set_one_element` | S |
| 5. Get codegen — 2 reads + Block | `src/parser/mod.rs::get_val` | XS |
| 6. Native emit special-case for `fn_ref_field_read` Block extension | `src/generation/emit.rs` | XS |
| 7. Same-store closure deep-copy | `src/parser/mod.rs` + new runtime helper | M |
| 8. Cleanup `OpSetFnRef` calls + dead code | `src/parser/mod.rs` (delete the 2 broken branches) | XS |
| 9. Regression tests (4 new) | `tests/issues.rs` | S |
| 10. Doc — close P196 in PROBLEMS.md, retire 4d.C in plan README, CHANGELOG entry | docs | XS |

**Total: M (1–2 days).**  Step 7 dominates; without it, lambdas-in-fields work for same-store cases but silently break across par worker boundaries.  All other steps are mechanical.

## Risk

- **Same-store closure copy** is the only piece that touches a
  semantic the existing tests don't exercise.  Mitigation: write
  the cross-store regression test FIRST so the implementation has a
  concrete target.
- **Storage layout change** (4 → 16 bytes per fn-ref field) means
  every existing struct with a fn-ref field grows.  No tests
  assert byte-level field offsets today — the recently-landed
  `p4d_*` regression tests check behavioural invariants only.  Low
  risk.
- **Lambda + par interaction** (closure in worker output → main):
  rebases via existing `StoreRebase` walk at par-stitch time
  (@PLAN06 phase 2).  Phase 4d.C explicitly does NOT solve par +
  capturing-lambda inputs (`par_vec_of_fns_input_t4`); that needs
  separate phase-9 work on closure transport.

## Out of scope

- Capturing lambdas as par worker INPUTS (the
  `par_vec_of_fns_input_t4` cascade) — still phase 9.
- Cross-process / serialised closure transport — closures stay
  machine-local.
- Fn-ref equality / comparison — `==` on fn-refs already works at
  the d_nr level; the closure half doesn't change that semantics.

## Verification

1. Build: `cargo build --release`, `cargo build --release
   --target wasm32-unknown-unknown --lib --no-default-features
   --features random`.
2. Suite: `cargo test --release --no-fail-fast` — every existing
   `p4d_*` test must pass; new regressions must pass.
3. Native parity: each new test runs in both `--interpret` and
   `--native` modes (the test harness already does this for
   regressions in `tests/issues.rs`).
4. Storage size check: `database.size(known_type_of_fn_ref_field)`
   returns 16 (was 4).  Optionally add a layout-test entry under
   `database/types.rs::layout_tests`.
5. Cross-store guard: a regression test that constructs a lambda in
   a function-local closure record, assigns it to a struct field
   (host in different store), then calls the fn-ref through the
   field after the source scope exits.  Must not UAF.

## Cross-references

- [04-typed-input-output.md](04-typed-input-output.md) — phase 4d
  parent plan (4d.A worker-input typed dispatch, 4d.B keyed-input
  materialise, this is **4d.C**).
- [04d-followups.md](04d-followups.md) — three remaining 4d
  follow-up issues (P194 tuple-field reassignment, P195 lexer
  ambiguity, P196 tuple-of-fn-ref native).  P196 is **subsumed by
  this plan** — once 4d.C lands, the storage layout matches the
  native rep and the projection hack disappears.
- [PROBLEMS.md § 196](../../../PROBLEMS.md) — the open issue this
  plan closes.
- `src/parser/vectors.rs::parse_fn_ref` / `parse_lambda` — the two
  fn-ref construction paths whose IR shape this plan reads.
- `src/state/codegen.rs::Value::FnRef` — the existing 20-byte stack
  representation (`OpConstInt(d_nr)` + `OpVarRef(closure)`) that
  remains unchanged.
- `src/generation/mod.rs::rust_type` line 298 (`Type::Function ⇒
  "(u32, DbRef)"`) — the native representation that defines the
  storage layout target.
