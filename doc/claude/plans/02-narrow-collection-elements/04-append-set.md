<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 4 — Append / Insert / Set / file-write paths

**Status:** blocked by Phase 3.

**Goal:** every write-side opcode (`OpAppendVector`,
`OpInsertVector`, `OpSetVector`, `OpClearVector`, `OpRemoveVector`)
honours the narrow element stride for narrow vectors.  Plus: the
binary-file write path already uses `database.size(elem_tp)` and
should auto-honour the narrow type once Phase 2 creates it; verify
end-to-end that `f += b.v` for `vector<i32>` writes exactly 4 bytes
per element.

After this phase, `lib/graphics/src/glb.loft::glb_write_indices` can
revert to the natural `glb_idx_buf() -> vector<i32>` form.

---

## Sites to audit

### Parser-side emission of write opcodes

`rg 'OpAppendVector|OpInsertVector|OpSetVector|OpClearVector|OpRemoveVector' src/parser/`:

- `src/parser/collections.rs` — primary emission site for vector
  literals and compound assignments.
- `src/parser/expressions.rs::parse_assign_op` — rewrites field =
  vec_var to OpClearVector + OpAppendVector (P152 fix).
- `src/parser/objects.rs::handle_field` — struct-construction
  vector initialisers.
- `src/parser/builtins.rs` — parallel-for and friends.

For each, find the `elm_size` / `size` computation and apply the
same change as Phase 3: prefer `Type::Integer`'s forced_size over
the base-integer heuristic.

Use the `Data::element_width(elem_tp, nullable)` helper introduced
in Phase 3.  Single-source of truth for element stride.

### Runtime bytecode

`rg 'OpAppendVector|fn append_vector|fn insert_vector|fn set_vector' src/fill.rs src/state/` returns the runtime handlers:

- `src/fill.rs::append_vector` — reads the element size from the
  code stream (baked at emit time).
- `src/fill.rs::insert_vector` — same shape.
- `src/fill.rs::set_vector` — same.
- `src/fill.rs::clear_vector`, `remove_vector` — take size from code.

Conclusion: **runtime is already correct** as long as the parser
sites emit the right size.  No change in fill.rs.

### Low-level helpers

`src/vector.rs::vector_append`, `vector_finish` take a `size: u32`
parameter from callers.  Audit each caller:

`rg 'vector::vector_append' src/`:
- `src/state/io.rs::append_copy` — uses `self.database.size(ctp)`
  where `ctp = self.database.content(tp)` (the vector's content
  type from the vector's db_tp).  Auto-honours narrow content
  because `database.size(Parts::Int)` returns 4.  ✓ NO CHANGE.
- `src/database/io.rs:*` — parser initialisation helpers; uses
  `self.size(c)` from the database.  ✓ NO CHANGE.
- `src/database/structures.rs:41/118/176` — generic vector ops.

Confirm nothing hard-codes 4 or 8.

### Native codegen

`src/generation/emit.rs` / `src/generation/mod.rs` — vector append
code emission.  Audit for cached sizes; apply
`Data::element_width` where needed.

`src/codegen_runtime.rs::cr_append_vector` and siblings — runtime
helpers for `--native` mode.  These use `stores.size(elem_tp)`
which auto-honours narrow content.  ✓ NO CHANGE.

### File I/O — binary write

`src/state/io.rs::assemble_write_data` line 121-143:

```rust
} else if let Parts::Vector(elem_tp) = &self.database.types[db_tp as usize].parts {
    let elem_tp = *elem_tp;
    // ...
    let elem_size = u32::from(self.database.size(elem_tp));
    for i in 0..length {
        let elem = DbRef { ..., pos: 8 + elem_size * i };
        self.database.read_data(&elem, elem_tp, little_endian, &mut data);
    }
}
```

`database.size(elem_tp)` returns 4 for `Parts::Int`-typed narrow
content → reads 4 bytes per element → emits 4 bytes per element.
✓ **This path auto-works once Phase 2 lands.**

Actually VERIFY `read_data` for `Parts::Int`:
- `src/state/io.rs::read_data` (or `database/io.rs`) should emit 4
  bytes for a `Parts::Int` source.  That's the same code that
  struct-field narrow integers use today, so it's exercised.

---

## Expected revert in glb.loft

Once Phase 4 is green, this revert should land as the final commit
of the initiative:

```loft
// lib/graphics/src/glb.loft — restore the natural form

fn glb_idx_buf(tris: vector<mesh::Triangle>) -> vector<i32> {
  result: vector<i32> = [];
  for t in tris {
    result += [t.a as i32, t.b as i32, t.c as i32];
  }
  result
}

// Callers:
//   f += glb_idx_buf(m.triangles);
```

Verify:
- `lib/graphics/tests/glb.loft` passes.
- `lib/moros_render/tests/geometry.loft::test_map_export_glb_header`
  passes (the GLB header's `total_len` matches the real file size).

---

## Test matrix

Extend `p184_*` from Phase 3 with write-side coverage:

| Test                                  | Assertion                                    |
|---------------------------------------|----------------------------------------------|
| `p184_vector_i32_append_then_read`    | Append + index round-trip.                   |
| `p184_vector_i32_binary_write_size`   | `f += b.v` writes `len × 4` bytes.           |
| `p184_vector_integer_binary_write`    | Control: `vector<integer>` writes `len × 8`. |
| `p184_vector_u8_binary_write`         | `vector<u8>` writes `len × 1` bytes.         |
| `p184_glb_natural_form`               | Revert glb.loft workaround, assert GLB size matches header. |

---

## Risks

- **Parallel vector write paths**.  `src/parser/builtins.rs::par`
  (parallel-for) computes result-vector element sizes independently
  from the main parser path.  Audit the `elem_size` argument to
  `OpParallelFor` and similar.
- **Vector-constant init**.  `src/compile.rs::build_const_vectors`
  writes initial values at compile time via `OpSet*` calls.  If the
  const-vector's content is narrow, these writes must also be
  narrow — confirm via a test like `pub C: vector<i32> = [1, 2, 3];`
  and assert `len(C) == 3` + `C[0] == 1` at runtime.

---

## Acceptance

- [ ] All Phase 3 tests remain green.
- [ ] New p184_* write-side tests green.
- [ ] glb.loft workaround reverted + still passes.
- [ ] `make test-packages` green for graphics, moros_render.

---

## Rollback

If a write-path bug surfaces mid-phase:
- If the bug is isolated to one site (e.g. parallel-for), keep the
  general fix but restore the pre-fix code at that specific site.
- If the bug is systemic (e.g. every OpSetInt4 call is wrong),
  revert Phase 4 AND Phase 3 AND Phase 2.  Return to the documented
  workaround in `glb.loft`.
