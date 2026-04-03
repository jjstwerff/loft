# Runtime Optimisation Opportunities

This document audits the interpreter runtime for concrete performance improvements,
weighing impact against implementation cost and maintainability.

## Contents
- [Open opportunities](#open-opportunities)
- [Not worth changing](#not-worth-changing)
- [Open — recommended priority order](#open--recommended-priority-order)

Completed optimisations (debug_assert, clone removal, Arc bytecode sharing, LLRB free-list)
are recorded in CHANGELOG.md.

---

## Open opportunities

### 1. `Stores::types` and `Stores::names` cloned for every worker

**File:** `database.rs:1541-1561`

`clone_for_worker` copies:

- `types: self.types.clone()` — `Vec<Type>`, read-only after compilation
- `names: self.names.clone()` — `HashMap<String, u16>`, read-only after compilation

Both are pure metadata that no worker modifies.  Wrapping them in
`Arc<Vec<Type>>` and `Arc<HashMap<String, u16>>` would reduce the per-worker
clone to two atomic-ref-count increments.

For a program with 200 types and a 500-entry name map the savings are small in
absolute bytes, but the pattern becomes significant if the type system grows or
if hundreds of parallel calls are made.

**Impact:** Low-Medium — mainly prevents future scaling problems
**Cost:** Medium — field types change throughout `database.rs`; some methods need `Arc::make_mut` if mutation is ever needed before `clone_for_worker` is called
**Verdict:** Defer until parallel usage grows; note the shape of the fix here

---

## Not worth changing

| Pattern | Reason |
|---|---|
| `State` HashMap fields (`stack`, `vars`, `calls`, `types`, `line_numbers`) | Only accessed in debug/dump functions, not in the hot execute loop |
| `WorkerProgram` channel + batching in `parallel.rs` | `Vec::with_capacity(end-start)` is already exact; no reallocation |
| `calc.rs` BTreeMap for struct layout | Compile-time only; immeasurable runtime effect |
| `library_names: HashMap<String, u16>` | Queried during compilation, not execution; worker states leave it empty |
| Function pointer dispatch table in `fill.rs` | Already optimal for an interpreter; JIT is the next step |

---

## Open — recommended priority order

| # | Change | File(s) | Effort | Impact |
|---|--------|---------|--------|--------|
| 1 | `Arc` for `Stores::types` / `names` | `database.rs` | Medium | Low–Med |
| 2 | O8.1b: packed bytes in bytecode | `vector.rs`, `state/mod.rs` | Medium | High |
| 3 | O8.3: zero-fill struct defaults | `parser/objects.rs` | Small | Low–Med |

---

## O1 Superinstruction Peephole — Design Notes (deferred)

The infrastructure for superinstructions is in place but the peephole rewriting
pass is deferred to a future release.  This section documents the design for
the implementor.

### What exists

- **Opcodes registered** in `default/01_code.loft`: `OpSiLoad2AddStore`,
  `OpSiLoadConstAddStore`, `OpSiLoadConstCmpBranch`, `OpSiLoad2CmpBranch`,
  `OpSiLoadConstMulStore`, `OpSiLoad2MulStore`, `OpNop`.
- **State stubs** in `src/state/mod.rs`: delegation methods that call `nop()`.
  Replace these with the real implementations below.
- **`fill.rs` auto-generated** with the opcodes in the OPERATORS array.
- **`build_opcode_len_table()`** in `src/compile.rs`: computes instruction
  byte-lengths from operator definitions — survives renumbering.
- **`opcode_by_name()`** in `src/compile.rs`: resolves opcode numbers by name.
- **`fill_rs_up_to_date`** CI test: asserts `src/fill.rs` matches the generated
  version — prevents drift when `01_code.loft` changes.

### The stack-relative operand problem

`get_var(pos)` computes `stack_base + stack_pos - pos`.  Each `VarInt` pushes
4 bytes, advancing `stack_pos`.  The superinstruction runs without intermediate
pushes, so the second operand sees the wrong `stack_pos`.

**Arithmetic for `VarInt(a) VarInt(b) AddInt PutInt(c)` at initial SP:**

| Instruction | stack_pos | Address accessed |
|-------------|-----------|-----------------|
| VarInt(a) | SP | base + SP - a |
| VarInt(b) | SP+4 | base + SP + 4 - b |
| AddInt | SP+8→SP+4 | (pops 2, pushes 1) |
| PutInt(c) | SP+4→SP | base + SP + 4 - c |

The superinstruction at SP (no pushes):
- `get_var(a)`: base + SP - a ✓
- `get_var(b)`: base + SP - b ✗ (should be base + SP + 4 - b)
- `put_var(c)`: base + SP + 4 - c ✓ (put_var adds sizeof(T) internally)

**Fix:** adjust `b' = b - 4` in the peephole rewriter.  Then `base + SP - (b-4) = base + SP + 4 - b`. ✓

**Guard:** skip the pattern when `b < 4` (would underflow).

### Real implementations for State methods

Replace the `nop()` stubs with:

```rust
pub fn si_load2_add_store(&mut self) {
    let a = *self.code::<u16>();
    let b = *self.code::<u16>();  // pre-adjusted: b' = b - 4
    let c = *self.code::<u16>();
    let va = *self.get_var::<i32>(a);
    let vb = *self.get_var::<i32>(b);
    self.put_var(c, crate::ops::op_add_int(va, vb));
}
// Same pattern for si_load2_mul_store.
// For const variants: k is a literal (no adjustment).
// For cmp+branch: si_load2_cmp_branch reads i16 offset, branches if va >= vb.
```

### Peephole rewriter

Add `PeepholeCtx` to `src/compile.rs` that:
1. Builds opcode-length table via `build_opcode_len_table(data)`
2. Resolves opcodes by name via `opcode_by_name(data, name)`
3. Scans each function's bytecode as a sliding 4-instruction window
4. Matches patterns with exact length guards (l0==3, l1==3, l2==1, l3==3)
5. Rewrites in-place with adjusted operands, fills excess bytes with OpNop
6. **Skips default library functions** (`data.def(d_nr).position.file.starts_with("default/")`)

### Known issue: default library corruption

Running the peephole on default library functions causes `issue_84` tests
(recursive merge sort) to fail with "Unknown record" errors.  Root cause:
the default library uses patterns where the VarInt operands interact with
store-relative addressing in ways the simple b-4 adjustment doesn't cover
(possibly involving RefVar parameters or OpCreateStack pushes between the
matched instructions).

**Mitigation:** skip default library functions.  They're already fast
(hand-optimised `#rust` templates).  Only user functions benefit from
superinstructions.

### Adjustments per pattern

| Pattern | a | b/k | c/off | Super size |
|---------|---|-----|-------|------------|
| `VarInt VarInt {Add\|Mul}Int PutInt` | a | b-4 | c | 7 bytes |
| `VarInt ConstInt {Add\|Mul}Int PutInt` | a | k | c | 9 bytes |
| `VarInt VarInt LtInt GotoFalse` | a | b-4 | i16 offset | 7 bytes |
| `VarInt ConstInt LtInt GotoFalse` | a | k | i16 offset | 9 bytes |

Branch offset for cmp patterns: original `goto_false` offset is i8 relative
to `pc3+2`.  Super offset is i16 relative to `pc+7` (or `pc+9` for const).
Compute: `new_off = (pc3 + 2 + old_off) - (pc + super_size)`.

---

## See also
- [PERFORMANCE.md](PERFORMANCE.md) — Benchmark results, root-cause analysis, and detailed designs for O1–O7 (superinstructions, stack pointer cache, native collection emit, purity analysis)
- [PLANNING.md](PLANNING.md) — Priority-ordered backlog
- [INTERNALS.md](INTERNALS.md) — `src/parallel.rs`, `src/store.rs`, `src/state/` implementation details

### 2. O8: Constant data initialisation (delivered 2026-04-02)

**Files:** `src/const_eval.rs`, `src/vector.rs`, `src/fill.rs`, `src/parser/vectors.rs`

Three optimisations delivered:

- **O8.1a** `OpPreAllocVector`: pre-allocates vector capacity for known-size
  literals, eliminating all `store.resize()` calls.  One new opcode (replaced
  unused `OpNop` slot).
- **O8.5** Constant comprehension unrolling: `[for i in 0..N { expr(i) }]`
  unrolled at compile time when bounds and body are const-evaluable.  10k limit.
- **`const_eval()`** module: compile-time constant folder for arithmetic, casts,
  comparisons, boolean ops across all numeric types.

**Impact:** For a 20-element constant vector, eliminates 1-2 resize allocations.
For constant comprehensions, eliminates the entire runtime loop.

Full design: [CONST_DATA.md](CONST_DATA.md).

---
