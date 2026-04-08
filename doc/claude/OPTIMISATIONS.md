
# Runtime Optimisation Opportunities

This document audits the interpreter runtime for concrete performance improvements,
weighing impact against implementation cost and maintainability.

## Contents
- [Open opportunities](#open-opportunities)
- [Not worth changing](#not-worth-changing)
- [Open — recommended priority order](#open--recommended-priority-order)
- [W — WASM Game Efficiency](#w--wasm-game-efficiency)

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

## W — WASM Game Efficiency

> **Scope note:** Production games compiled with native codegen (glutin/winit/OpenGL) do
> not go through the bytecode interpreter at all, so interpreter-level tweaks do not
> apply to them.  Items marked **interpreter only** are relevant for browser-hosted games
> that run loft bytecode inside the WASM interpreter.  Items marked **all targets** apply
> to every deployment path.

---

### W1. GL call overhead reduction *(WASM browser path only)*

**Status:** Not started; original design revised after reading actual code
**Impact:** Medium — boundary crossing is not the dominant cost; see analysis
**Effort:** varies by sub-item (JS-only to Rust+JS)

#### Actual cost structure (from `src/wasm_gl.rs` and `lib/graphics/js/loft-gl.js`)

Three costs apply per GL call, in decreasing importance:

**Cost A — `getUniformLocation` called every frame** (`loft-gl.js:268–294`)

```javascript
gl_set_uniform_mat4(program, name, mat) {
    const loc = gl.getUniformLocation(programs[program], name);  // ← every call
    if (loc) gl.uniformMatrix4fv(loc, false, mat);
}
```

`gl.getUniformLocation` is a driver-level hashtable lookup.  It is called on
every `gl_set_uniform_*` call, every frame.  A scene with 5 uniforms × 2
draw calls = 10 location lookups/frame × 60 fps = 600 lookups/second.
This is the largest avoidable per-frame cost.

**Cost B — `js_sys::Array` allocated per call** (`wasm_gl.rs` — every `wgl_*`)

Every function creates a new JS Array to box its arguments before calling
`host_call_raw`.  Even argument-free calls like `wgl_poll_events` and
`wgl_swap_buffers` do `js_sys::Array::new()`.  This is GC heap allocation
on every call — JS GC pressure across ~50–200 calls/frame.

**Cost C — WASM→JS boundary crossing**

In modern browsers (V8/SpiderMonkey), a WASM-to-JS call costs ~10–50 ns.
At 200 calls/frame × 60 fps × 50 ns = ~0.6 ms/second.  This is real but
not dominant — it is smaller than Cost A and B.

**`gl.flush()` is a no-op.** `gl_swap_buffers` in JS just calls `gl.flush()`,
which browsers treat as advisory.  The actual frame present is driven by
`requestAnimationFrame` on the JS side.  A command-buffer that flushes at
`gl_swap_buffers` does not change GPU submission timing at all.

#### Fix A: cache uniform locations in JS (highest impact, trivial effort)

In `loft-gl.js`, add a two-level cache keyed by `(programIdx, name)`:

```javascript
const uniformCache = new Map();  // program_idx → Map(name → WebGLUniformLocation)

function getUniformLoc(program, name) {
    let prog_map = uniformCache.get(program);
    if (!prog_map) {
        prog_map = new Map();
        uniformCache.set(program, prog_map);
    }
    let loc = prog_map.get(name);
    if (loc === undefined) {
        loc = gl.getUniformLocation(programs[program], name);
        prog_map.set(name, loc);
    }
    return loc;
}
```

Replace all `gl.getUniformLocation(...)` calls with `getUniformLoc(program, name)`.
Invalidate the cache entry in `gl_delete_shader`.

**Impact:** Eliminates 600+ `getUniformLocation` lookups/second.
**Effort:** ~15 lines of JS.  No Rust changes.

#### Fix B: eliminate `js_sys::Array` per call (medium impact, medium effort)

Replace the `host_call_raw(name, &Array)` pattern in `wasm_gl.rs` with direct
`wasm_bindgen` imports — one `#[wasm_bindgen]` extern declaration per GL
function.  Arguments pass as typed primitives; no Array boxing.

```rust
#[cfg(feature = "wasm")]
#[wasm_bindgen(module = "/lib/graphics/js/loft-gl.js")]
extern "C" {
    fn gl_draw(vao: i32, vertex_count: i32);
    fn gl_clear(color: i32);
    fn gl_use_shader(program: i32);
    // etc.
}
```

Calling `gl_draw(vao, count)` directly costs one WASM→JS call with no
allocations.  The current path costs one `Array::of2` (JS heap allocation) +
one `host_call_raw` call on top.

**Impact:** Eliminates ~50–200 JS Array allocations/frame; reduces GC pressure.
**Effort:** Medium — all `wgl_*` functions in `wasm_gl.rs` need updating;
`host_call_raw` path replaced with typed imports.

#### Fix C: command-buffer batching (low additional impact after A+B)

After fixes A and B, the remaining WASM→JS boundary cost is ~0.6 ms/second
(200 calls × 50 ns × 60 fps).  Batching all draw calls into a typed buffer
and flushing once at `gl_swap_buffers` could halve this — saving ~0.3 ms/s.
At 60fps this is ~5 μs/frame, which is below measurement noise for most games.

**Verdict:** Not worth implementing once A and B are done.  May become relevant
for scenes with 1 000+ draw calls/frame (deferred rendering, particle systems
with per-particle draw calls).

#### Mat4 conversion cost (separate concern)

`wgl_set_uniform_mat4` calls `extract_f64_as_f32_vector` which allocates a
`Float32Array` and converts 16 f64 values to f32.  This is unavoidable given
the interpreter stores `float` as f64 and WebGL uniforms are f32.  It costs
one allocation + 16 multiply/cast per mat4 uniform per frame.  Not addressable
without changing the loft `float` type or adding a `single` uniform path.

#### Priority summary for W1

| Fix | Impact | Effort | Where |
|---|---|---|---|
| **A** — cache uniform locations | **High** — eliminates most per-frame GL overhead | Trivial | `loft-gl.js` only |
| **B** — direct wasm_bindgen imports | Medium — eliminates Array GC pressure | Medium | `wasm_gl.rs` |
| **C** — command-buffer batching | Low (after A+B) | Medium | `loft-gl.js` |

Implement A first.  It costs 15 lines and eliminates the largest real cost.
Implement B if profiling shows GC pauses from Array allocation.  Skip C unless
draw-call count exceeds ~500/frame.

---

### W2. Game object store pooling *(all targets)*

**Status:** Not started
**Impact:** Low — current S29 reuse already eliminates heap allocation; remaining
savings are minimal (see analysis below)
**Effort:** Medium–High — pool infrastructure, annotation, codegen hook

#### Why the impact is lower than it first appears

The S29 bitmap reuse (delivered 0.8.3) means freed stores are already
reused without `Store::new()` or heap allocation.  The actual cost of
`database_named()` when a free slot exists is:

| Step | Cost | Notes |
|---|---|---|
| `find_free_slot()` | ~1–2 ns | `trailing_zeros()` on first non-zero `u64` — O(1) |
| `store.unlock()` | trivial | single bool write |
| `store.init()` | ~10–30 ns | writes SIGNATURE + free header + **`claims.clear()` + `claims.insert(PRIMARY)`** |
| 3 field writes | trivial | `free`, `created_at`, `last_op_at` |
| `clear_free_bit` | trivial | single bit op |
| `store.claim(size)` | ~10–20 ns | `generation++`, LLRB lookup (empty → scan), **`claims.insert(pos)`** |

The dominant cost is the `HashSet` operations inside `init()` and `claim()` —
not the bitmap scan.  A free-list pool as designed would replace `find_free_slot()`
with a Vec pop, saving the trivial steps but **leaving `init()` and `claim()`
unchanged**.  Net saving per allocation: ~2–5 ns out of ~30–60 ns total.

#### The only path to real savings: eliminate `init()` and `claim()`

For plain-data structs (no owned `text`, `vector`, or `reference` fields),
the entire store content is overwritten on every use.  We can:

1. At game start, `claim()` a fixed record in each pool store once.
2. Store the resulting `DbRef` template (with pre-known `rec` and `pos`).
3. On alloc: pop from free-list + **memset the struct's byte range** to zero.
4. On free: push back to free-list (no `init()`, no LLRB, no HashSet).

This eliminates the HashSet entirely and reduces alloc to a Vec pop + `memset`.
For a 40-byte struct: ~3 ns vs ~50 ns — a genuine 15× speedup on the
alloc/free cycle.

**Constraint:** Only valid for structs with no owned fields.  Structs with
`text` or `vector` fields still need `remove_claims()` before return and
`init()` on reuse, leaving no meaningful saving over S29 reuse.

#### Design (plain-data pool only)

```rust
// src/database/pool.rs  (new file)

/// Pool for plain-data structs: no owned text, vector, or reference fields.
/// Pre-claims a fixed record in each store; alloc/free is Vec pop/push + memset.
pub struct PlainPool {
    /// Pre-initialized DbRef for each slot (store_nr, rec=1, pos=8).
    templates: Vec<DbRef>,
    /// Free-list stack of available slot indices into `templates`.
    free_list: Vec<u16>,
    /// Byte size of the struct (for memset).
    struct_bytes: u32,
}
```

On `pool_alloc`:
```rust
let slot = pool.free_list.pop()?;
let dbref = pool.templates[slot as usize];
// Zero the struct region in the backing store.
let store = &mut self.allocations[dbref.store_nr as usize];
unsafe {
    store.ptr.add((dbref.rec * 8 + dbref.pos) as usize)
         .write_bytes(0, pool.struct_bytes as usize);
}
Some(dbref)
```

On `pool_free`: push slot back, no cleanup needed (struct is plain data).

#### When to use a pool vs S29 reuse

| Object type | Fields | Savings | Verdict |
|---|---|---|---|
| Particle, Bullet, Tile | plain numeric/bool | ~15× alloc/free | Worth pooling |
| Enemy, Item with name text | owns `text` | ~0× | Not worth it |
| Any struct | mix | depends | Profile first |

#### Loft annotation

```loft
struct Particle #pool 1000 {
    x: float
    y: float
    vx: float
    vy: float
    life: float
    alive: boolean
}
```

Compiler (`src/compile.rs`): reject `#pool` on structs with owned fields
(error at compile time, not runtime).

#### Files to change (if pursued)

| File | Change |
|---|---|
| `src/database/pool.rs` | New: `PlainPool` + alloc/free |
| `src/database/mod.rs` | Add `plain_pools`, `pool_base`; alloc/free helpers |
| `src/database/allocation.rs` | `database()` checks pool first for plain-pool types |
| `src/parser/definitions.rs` | Parse `#pool N`; reject on owned-field structs |
| `src/compile.rs` | Emit pool init; validate plain-data constraint |
| `src/state/codegen.rs` | Route `OpDatabase` for pooled types |
| `src/fill.rs` | Route `op_free_*` for pool DbRefs |

#### Recommendation

Given that S29 already eliminates heap allocation, the priority of W.G2 should
be reconsidered.  Profile a real game workload first.  If `database_named` does
not appear in a profiler trace, skip this entirely.  If it does, implement the
plain-data memset variant only — the general pool (with `init()` + `claim()`)
saves nothing meaningful over what already exists.

---

### W3. Frame-aware dispatch loop *(interpreter only)*

The main execute loop checks `if self.database.frame_yield { return; }` after
**every** opcode.  Only `gl_swap_buffers` ever sets this flag; all other opcodes
pay a branch test for nothing.

**Opportunity:** Move the `frame_yield` check inside the `gl_swap_buffers` handler
and remove it from the outer loop.  Under the `wasm` feature flag, compile a
variant of the dispatch loop that does not test `frame_yield` per opcode.

```rust
// current (src/state/mod.rs — execute loop)
OPERATORS[op as usize](self);
if self.database.frame_yield { return; }   // paid 50 k+ times per frame

// proposed
OPERATORS[op as usize](self);
// frame_yield check moved into wgl_swap_buffers handler only
```

**Impact:** Low–Medium (eliminates one branch per opcode; ~2–5% on tight loops)
**Effort:** Small
**Status:** Not started

---

### W4. Opcode table redesign to unblock superinstructions (O1) *(interpreter only)*

The opcode table is full at 254/256 entries, blocking O1 indefinitely.

**Opportunity:** Introduce a two-byte escape prefix for opcodes 240–255.  This reclaims
the 16-slot range for superinstructions and allows O1 to proceed (see O1 design notes
above).

**Impact:** Very High (40–60% recovery of tight-loop slowdown based on PERFORMANCE.md estimates)
**Effort:** Very High (parser, codegen, all opcode handlers, `build_opcode_len_table`,
`fill_rs_up_to_date` CI test all require changes for variable-width instructions)
**Status:** Deferred post-1.0; too disruptive during stability focus

---

### W — Priority summary

| # | Item | Targets | Effort | Impact | Design |
|---|------|---------|--------|--------|--------|
| W1A | Cache uniform locations in loft-gl.js | WASM browser | Trivial | High | above |
| W1B | Direct wasm_bindgen imports (no Array per call) | WASM browser | Medium | Medium | above |
| W1C | Command-buffer batching | WASM browser | Medium | Low (after W1A+B) | above |
| W2 | Game object store pooling (plain-data only) | All | Medium–High | Low (S29 already reuses stores; savings only for plain-data structs) | above |
| W3 | Frame-aware dispatch | **Interpreter only** | Small | Low–Med | above |
| W4 | Opcode redesign → O1 | **Interpreter only** | Very High | Very High | above |

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
