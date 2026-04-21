<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 3 — Read path honours narrow element size

**Status:** blocked by Phase 2.  **Hardest phase in the plan — but
once mapped, smaller than it looks.**

**Goal:** every compile-time site that emits `OpGetVector` (or
equivalent) uses the actual vector's element stride, and the
subsequent scalar-read op (`OpGetInt4` / `OpGetShort` / `OpGetByte`)
matches.

---

## The good news — `get_val` already knows how

`src/parser/mod.rs::get_val` (line 1569) already dispatches on
width when given an alias:

```rust
Type::Integer(min, _, _) => {
    let s = self.data.forced_size(alias)
        .unwrap_or_else(|| tp.size(nullable));
    if s == 1       { self.cl("OpGetByte",  &[code, p, Value::Int(*min)]) }
    else if s == 2  { self.cl("OpGetShort", &[code, p, Value::Int(*min)]) }
    else if s == 4  { self.cl("OpGetInt4",  &[code, p]) }
    else            { self.cl("OpGetInt",   &[code, p]) }
}
```

The scalar-read op for 4-byte narrow integers (`OpGetInt4`) is
already plumbed to fill.rs and the native backend.  The policy is
already "prefer forced_size over bounds heuristic".  **What's
missing is the plumbing — callers that pass `alias = u32::MAX`
even when the type is a narrow alias.**

---

## The three broken emission sites

### Site 1 — field-access index (`b.v[i]`)

`src/parser/fields.rs:402-463` (`parse_vector_index`):

```rust
let elm_td = self.data.type_elm(etp);                   // "integer" def_nr
let known = self.data.def(elm_td).known_type;           // 0
let elm_size = i32::from(self.database.size(known));    // 8 ← WRONG
// ...
*code = self.cl("OpGetVector", &[code.clone(), Value::Int(elm_size), p]);
if self.database.is_base(known) {
    *code = self.get_val(etp, true, 0, code.clone(), u32::MAX);
    //                                              ^^^^^^^^^^ ← WRONG
}
```

Two bugs on the same site:
- `elm_size = 8` → `OpGetVector` strides by 8.
- `alias = u32::MAX` → `get_val` doesn't see forced_size → emits `OpGetInt` (8-byte read).

### Site 2 — iterator setup (`for x in b.v`)

`src/parser/control.rs:1623`:

```rust
let get = self.cl("OpGetVector", &[Value::Var(v), elm_size.clone(), idx]);
```

Same shape — `elm_size` baked wrong upstream.

### Site 3 — chained field access (`base.field.v[i]`)

`src/parser/fields.rs:458` — same as Site 1, same two bugs.

---

## Evaluated solutions

### ❌ Option A: thread alias through every call chain

Add an extra `alias_d_nr: u32` param to every parser helper between
type resolution and `get_val`.  ~10 call sites, invasive signature
changes, and fails for local variables (no Attribute → no alias).

### ❌ Option B: attach forced_size to `OpGetField`'s info block

Runtime opcode change.  Too heavy for this fix.

### ❌ Option C: precompute Type→db_tp map, consult db_tp's content

Requires a new data structure with cache invalidation; and `Type`
equality is fuzzy in places (e.g. Rewritten wraps).  Too much
machinery.

### ✅ Option D (RECOMMENDED): read forced_size directly from `Type::Integer`

Post-Phase-0, `Type::Integer` carries `Option<NonZeroU8>` as its
fourth field.  That IS the authoritative source of truth for
narrow integer widths.  Every emission site has access to the
content Type (`etp`); just read the fourth field.

**Change 1 — `get_val` falls back to Type's forced_size when alias
is absent** (`src/parser/mod.rs:1572`):

```rust
Type::Integer(min, _, _, forced_opt) => {
    let s = self.data.forced_size(alias)
        .or_else(|| forced_opt.map(NonZeroU8::get))
        .unwrap_or_else(|| tp.size(nullable));
    // ... same dispatch, unchanged
}
```

Now callers that pass `alias = u32::MAX` still get the right width
if the Type carries forced_size.

**Change 2 — `parse_vector_index` uses Type's forced_size for
elm_size** (`src/parser/fields.rs:410-412`):

```rust
let elm_size = if let Type::Integer(_, _, _, Some(forced)) = etp {
    i32::from(forced.get())
} else {
    let elm_td = self.data.type_elm(etp);
    let known = self.data.def(elm_td).known_type;
    i32::from(self.database.size(known))
};
```

Or wrapped as a helper on Parser (or Data):

```rust
fn elem_byte_width(&self, etp: &Type) -> u8 {
    if let Type::Integer(_, _, _, Some(n)) = etp {
        return n.get();
    }
    let elm_td = self.data.type_elm(etp);
    let known = self.data.def(elm_td).known_type;
    self.database.size(known)
}
```

**Change 3 — apply the same helper at Site 2 and Site 3.**

That's the entire Phase 3.  ~5 touched lines once a helper is in
place.

### Why Option D is better than the others

- **Single source of truth.**  The forced_size lives on Type; every
  consumer reads from the same place.  Future refactors don't
  drift.
- **Zero IR / runtime change.**  No new opcodes, no bytecode
  format changes.  Phase 3 ships as a pure parser diff.
- **Works for locals and returns.**  Because the forced_size is
  on Type, not Attribute, any Type-carrying context (parameter,
  local, return) auto-participates.
- **Trivially testable.**  Unit-test `elem_byte_width` against
  the Type variants directly.
- **Trivially revertable.**  The helper returns the same as the
  pre-Phase-3 code when forced_size is None → non-narrow vectors
  are unaffected.

### Key assumption: Type::Integer's forced_size survives

**This is the entire bet for Phase 3.**  The forced_size MUST
survive from the Phase 1 parse site (`parse_type`) to the Phase 3
read site (`parse_vector_index`).  If any intermediate pass strips
the fourth field, Phase 3 silently falls back to wide reads.

Audit sites where Type::Integer is reconstructed or cloned:

```bash
rg 'Type::Integer\([^)]+\)' src/ | rg -v 'Type::Integer\([^)]+,\s*(_|None|forced|Some)'
```

Lists every construction that DOESN'T propagate the fourth field.
For each, verify the code path: is it compiler-generated (where
`None` is correct) or does it copy an existing Type (where the
original's fourth field should be preserved)?

**Instrumentation to catch regressions**: in Phase 1 we recommended
a debug-assertion at `fill_database` comparing Attribute's
`alias_d_nr` → `forced_size` against Type's fourth field.  Add the
same shape of check in `get_val` to catch drift before it reaches
user programs:

```rust
// In debug builds, when both alias and Type carry forced_size,
// assert they agree.  Soft-warn via `debug_assert_eq!`.
```

---

## Native codegen

`src/generation/` — audit for element-size assumptions:

- `src/generation/mod.rs::vector_elem_rust_type` — returns the Rust
  type string for the element.  For Type::Integer with `max > i32::MAX`
  (the I64 case) returns "i64"; otherwise "i32".  **This already
  uses Type bounds, but it doesn't consult forced_size.**
  Narrow variants (`i16`, `u8`) need adjustment if they want the
  right Rust type in generated FFI signatures.  For PURE loft code
  (no FFI), the Rust side uses `DbRef` and reads through `stores.size()`
  which auto-honours narrow DB types, so this may not matter end-to-end.

  Check via a narrow-vector test running under `--native`.

- `src/generation/expressions.rs` — expression codegen for indexing.
  Likely dispatches on Type; audit for hard-coded `8` or "i64".

- `src/codegen_runtime.rs::cr_get_vector` / friends — runtime
  helpers for `--native`.  Use `stores.size(elem_tp)` which
  auto-honours narrow DB types.  ✓ no change.

---

## Test matrix

`tests/issues.rs::p184_*`:

| Test                                | Assertion                                               |
|-------------------------------------|---------------------------------------------------------|
| `p184_vector_i32_index_narrow`      | `b.v[0] == 1`.                                          |
| `p184_vector_i32_index_wide_control`| `vector<integer>[0]` still works, returns 8-byte value. |
| `p184_vector_u16_index`             | 2-byte stride.                                          |
| `p184_vector_u8_index`              | 1-byte stride.                                          |
| `p184_vector_i32_for_loop`          | `for e in b.v { sum += e }`.                            |
| `p184_vector_i32_nested_index`      | `outer.inner.v[i]` — Site 3.                            |
| `p184_vector_i32_in_native`         | Same program under `--native` — identical result.       |
| `p184_get_val_consistency`          | Internal test: `get_val` emits `OpGetInt4` for a        |
|                                     | Type with `Some(NonZeroU8::new(4).unwrap())`.           |

---

## Acceptance

- [ ] All `p184_*` tests green in both interpreter and native mode.
- [ ] Bytecode dump for a `vector<i32>` index shows `OpGetVector(., 4, .) + OpGetInt4`.
- [ ] Bytecode dump for a `vector<integer>` index shows `OpGetVector(., 8, .) + OpGetInt`.
- [ ] `lib/moros_render/tests/geometry.loft` still green (with
      glb_write_indices workaround still in place — Phase 4 removes
      it).

---

## Rollback

If Phase 3 hits a blocker (e.g. an audit finds that forced_size
gets stripped at an inlining / substitution pass), revert Phase 3
AND Phase 2 simultaneously — do NOT leave narrow storage with wide
reads in production.  See README.md "Ground rule #1 — all or
nothing".
