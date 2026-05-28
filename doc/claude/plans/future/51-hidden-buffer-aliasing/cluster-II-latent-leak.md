# Cluster II — Latent leak (interpret-only)

**Severity:** Slow leak under repeated calls; linear scaling (1 Canvas per iter, confirmed at 100 iters).  Not silent corruption — `LOFT_STORES=warn` catches it.  But cumulative cost in production loops (dryopea editor: one full-screen Canvas per frame).
**Affected probes:** 02, 03, 07, 11, 21, 25, 26 (7 probes)
**Backend asymmetry:** `--interpret` leaks; `--native` is clean.

## Mechanism — pinned via IR diff

### Reference probe 01 (canonical, CLEAN)

```loft
fn render_p(p: P) -> Canvas { cv = alloc_canvas(4, 5, p.tag); cv }
```

**Lowered IR**:

```
fn n_render_p(p:P, cv:Canvas) -> Canvas["cv"] {
  __ref_1(1):ref(Canvas) = null;
  [30] cv(0):ref(Canvas) = n_alloc_canvas(4i32, 5i32, OpGetInt(p(0), 0i32), cv(0));
                                                                              ^^^^^
                                                              S1 SUBSTITUTED: cv passed as hidden buffer
  OpFreeRef(__ref_1(1));
  return cv(0);
}
```

**S1 substitution visible:** the inner call's last arg is `cv(0)` (the outer hidden buffer parameter), NOT a fresh `__ref_local`.  S2 then skips the pre-Set OpFreeRef (because args contain Var(cv)).  The inner call writes the new Canvas directly into cv's slot.  No intermediate store; no leak.

### Problem probe 02 (double-set, LEAKS Canvas×6)

```loft
fn render_double(p: P) -> Canvas {
  cv = alloc_canvas(3, 3, p.tag);       // First Set
  cv = alloc_canvas(4, 5, p.tag + 1);   // Second Set (penultimate)
  cv
}
```

**Lowered IR**:

```
fn n_render_double(p:P, cv:Canvas) -> Canvas["cv"] {
  __ref_2(1):ref(Canvas) = null;
  __ref_1(1):ref(Canvas) = null;
  [34] cv(0):ref(Canvas) = n_alloc_canvas(3, 3, p.tag, __ref_1(1));
                                                       ^^^^^^^^^^
                                       FIRST Set: hidden buffer = __ref_1 (NOT cv)
  [35] cv(0):ref(Canvas) = n_alloc_canvas(4, 5, p.tag + 1, cv(0));
                                                           ^^^^^
                                       SECOND Set: S1 SUBSTITUTED (cv)
  OpFreeRefIfDistinct(__ref_1(1), cv(0));   <-- pair-free: skip if same store
  OpFreeRef(__ref_2(1));                    <-- always null, no-op
  return cv(0);
}
```

**S1 fires only on the SECOND Set** (the immediate penultimate).  The FIRST Set's call uses `__ref_1` as its hidden buffer (a fresh work-ref).

### The leak walk-through (per iter, with iter-0 store map)

1. **First call: `alloc_canvas(3, 3, tag, __ref_1)`.**
   - Caller (render_double's frame) has __ref_1 = null (initially).
   - alloc_canvas allocates `__ref_1`'s slot, writes a Canvas record:
     - `cv = Canvas { data: [], w: 3 }` — allocates a NEW VECTOR STORE for `data: []` (child store, let's call it `V_1`).
     - Loop appends 3 elements to V_1.
   - alloc_canvas returns `__ref_1`'s DbRef.
   - render_double's Set assigns `cv = __ref_1`.  **cv and __ref_1 now alias the same store.**
2. **Second call: `alloc_canvas(4, 5, tag+1, cv)`.**  S1-substituted: hidden buffer = cv = __ref_1 (same store).
   - alloc_canvas writes `Canvas { data: [], w: 4 }` INTO cv's existing store.
     - The Canvas record gets overwritten — its fields are reassigned in place.
     - **A NEW VECTOR STORE `V_2` is allocated for the new `data: []`.**
     - The Canvas record's `data` field is updated: previously pointed to `V_1`, NOW points to `V_2`.
     - **`V_1` is now orphaned — no reference points to it.**
   - Loop appends 5 elements to V_2.
3. **Scope exit:**
   - `OpFreeRefIfDistinct(__ref_1, cv)`: __ref_1 and cv are the same store; skip free.  Correct — caller will free its hidden buffer.
   - `OpFreeRef(__ref_2)`: __ref_2 is null; no-op.
4. **Per iter:** `V_1` (first call's vector store) is leaked.  6 iters → Canvas×6 leak.

**This matches the verified store trace** in `/tmp/probe02_stores.txt` — `max` grows by 1 per iter, totaling +6 across 6 iters.

## The shape signature for Cluster II

The body has at least one `Set(cv, Call(fn, args))` where:
- `cv` is the ref_return-promoted hidden buffer.
- The Set is NOT the immediate penultimate of `Var(cv)` (so S1 doesn't substitute).
- The inner call allocates a heap struct with at least one child store (Canvas's `data` vector, or any nested heap field).

When S1 doesn't fire:
- The call uses a fresh `__ref_local` as its hidden buffer.
- The first such call's child store(s) live in __ref_local's store.
- A subsequent assignment to cv (whether via a later Set or via the struct-overwrite of an aliased slot) **does not recursively free child stores of the now-overwritten record.**

## Why the seven probes fall in this cluster

| Probe | What breaks S1's penultimate-Set match | Iter-1 leak count |
|---|---|---|
| 02 double-set | TWO consecutive Sets; S1 fires on the second only | Canvas×6 (1 per iter) |
| 03 intervening-stmt | Single Set, but intervening `_ = p.tag * 2` displaces it from penultimate position | Canvas×6 |
| 07 explicit-return | `return cv;` (statement form).  `block_result`'s tail-type is Void → ref_return doesn't fire → S1 doesn't even reach preconditions | Canvas×6 (suspected; see open questions) |
| 11 conditional-reassign | Penultimate is the `if`, not the Set | Canvas×6 |
| 21 many-iters | Identical to 02 with 100 iters | Canvas×100 (linear scaling confirmed) |
| 25 cond-always | `if true { … }` — second Set fires every iter | Canvas×6 |
| 26 cond-never | `if false { … }` — second Set NEVER fires at runtime, yet still leaks | Canvas×6 (codegen-pattern-driven) |

**Probe 26's leak when the conditional never fires** is the strongest evidence that the leak is a CODEGEN-PATTERN property, not a runtime-control-flow property.  The mere presence of a conditional Set in the IR causes the per-iter Canvas leak, even when the conditional's body is unreachable.

## What native does correctly

Probe 02 on `--native` passes clean.  Native's codegen (per `src/generation/`) lowers `Set(cv, expr)` to a Rust statement that uses ownership / `Drop` semantics:

- The new value's record is computed.
- The OLD value's record is dropped (Rust's destructor runs recursively, freeing child stores).
- The slot is updated.

This is automatic — Rust's drop handles the child-store recursion.  The interpret backend has no equivalent recursive-drop mechanism; each Set updates the slot but doesn't recursively free.

## What we know vs. don't

| | Status |
|---|---|
| The IR difference between probe 01 and 02 | ✅ Visible in `/tmp/bc_01.txt` and `/tmp/bc_02.txt` |
| S1 fires on second Set only in probe 02 | ✅ Visible in IR (`__ref_1` vs `cv` arg) |
| The child-store-orphan mechanism | ✅ Hypothesized; consistent with the +1 per iter store trace |
| The exact opcode that overwrites without recursive-free | 🤔 Likely `OpDatabase` (claiming the existing slot for the new record) followed by field-set ops that don't free old field values |
| Why probe 07 (explicit-return) leaks identically | 🤔 ref_return doesn't fire on Return-tail bodies; needs source reading at `parse_return` |
| Why probe 26 leaks when if-false never fires | 🤔 The conditional Set's CODEGEN affects buffer-protocol setup; the if's then-block contains Set(cv, ...) whose presence alone perturbs slot tracking |

## Investigation tasks

1. **Read `parse_return`** at `src/parser/control.rs:3108-3190` — verify the hypothesis that explicit-return bodies bypass `block_result`'s ref_return → S1 chain.
2. **Read `OpDatabase` + field-set codegen for Set-into-existing-Reference** in `src/state/codegen.rs` — find the path that handles `cv = Canvas { ... }` when cv is a hidden buffer.  Is there recursive-free of old field values?  Probably not.
3. **Read native's Set-Reference codegen** in `src/generation/` for comparison.  How does it handle the same pattern correctly?
4. **Look at OpFreeRefIfDistinct semantics** — could it be extended to recursively free child stores when the witness and buffer alias?

## Fix surface

**(a) Recursive child-store free on Set-into-Reference.**  When `Set(cv, NewRecord)` where cv already points to a record, recursively free the OLD record's child stores before overwriting.  This is what native does implicitly via `Drop`.  Effort: M — need to walk the type descriptor and free each reference field; risk: over-free of fields that are aliased elsewhere (need careful semantics).

**(b) Extend S1 to cover more shapes.**  S1 currently fires only on immediate-penultimate Set + Var(cv) tail.  Extending to multi-Set, intervening-stmt, conditional, explicit-return shapes would eliminate the need for a recursive-free in those shapes.  Effort: S per shape; ~M+ total.  Risk: each extension expands the precondition footprint; one new shape might collide with another fix's preconditions.

**(c) Path C — store refcount.**  Each store has a refcount; allocations inc, frees dec, child stores' refcounts are managed by the parent's lifecycle.  Eliminates the entire class.  Effort: L (1-2 weeks).  Subsumes Cluster III too.

**Most likely best path: (c).**  The class is fundamentally about manual-free semantics not matching the deep ownership structure of struct values with reference fields.  Trying to patch each pattern (a) or (b) is whack-a-mole.  Path C makes the model correct by construction.
