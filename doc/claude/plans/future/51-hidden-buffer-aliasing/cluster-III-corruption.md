# Cluster III — Silent data corruption (interpret-only)

**Severity:** Worst data-integrity class — silent wrong-value reads with no warnings.  Only caught by per-iter assertions.
**Affected probes:** 04 (mixed-lit-call), 28 (only-conditional-set)
**Backend asymmetry:** `--interpret` corrupts; `--native` clean.

The two probes in this cluster have **different** symptoms and likely different root mechanisms.  Each is documented separately below.

---

## Probe 04 — mixed struct-literal then call

```loft
fn render_lit_then_call(p: P) -> Canvas {
  cv = Canvas { data: [], w: 1 };    // First Set — struct literal
  cv = alloc_canvas(4, 5, p.tag);    // Second Set
  cv
}
```

**Symptom:** At iter 2, `cv_c.data[0] = 1` (expected 2 = iter index `i`).  iter 2 reads iter-1's data.

**Lowered IR**:

```
fn n_render_lit_then_call(p:P, cv:Canvas) -> Canvas["cv"] {
  __ref_1(1):ref(Canvas) = null;
  [34] OpDatabase(cv(0), 66i32);            <-- FIRST Set: struct lit lowered as
  OpSetInt4(cv(0), 8i32, 0i32);             <-- OpDatabase + field-set ops
  OpSetInt(cv(0), 0i32, 1i32);              <-- (data field set to 0, w field set to 1)
  [35] cv(0):ref(Canvas) = n_alloc_canvas(4, 5, p.tag, __ref_1(1));
                                            <-- SECOND Set: passes __ref_1, NOT cv
                                            <-- S1 DID NOT SUBSTITUTE
  [36] OpFreeRefIfDistinct(__ref_1(1), cv(0));
  return cv(0);
}
```

### Why S1 didn't substitute (surprising)

S1's preconditions appear to match:
- Penultimate IS `Set(cv, Call(...))` ✓
- Tail IS `Var(cv)` ✓
- alloc_canvas has hidden buffer attr ✓
- args[i] is `Var(__ref_1)` (parser-internal name) ✓

But the IR shows the second call still uses `__ref_1` — S1 didn't fire.  The leading hypothesis: the struct-literal FIRST Set lowers to a different IR shape than expected.  Rather than producing a single `Value::Set(cv, ...)` node, it might lower to either:

1. **An `Insert`/`Block` wrapping multiple statements** (`OpDatabase + OpSetInt4 + OpSetInt`).  If this is l[0] and l[1] is the second Set, l[last-1] would be l[1] (the second Set with Call) and S1 SHOULD fire — but it doesn't.
2. **A SEQUENCE of statements directly added to l** (not wrapped).  Then l would be `[OpDatabase, OpSetInt4, OpSetInt, Set(cv, Call), Var(cv)]`, and `l[last-1]` is still the Set.  S1 should still fire.
3. **A different position relative to the second Set** — perhaps the struct-literal lowering inserts itself AFTER the second Set's IR is built, displacing the Set from the penultimate position.

The fact that the IR shows the second call un-substituted strongly suggests S1's `nrvo_collapse_tail_set` doesn't reach this Set, possibly because **l's layout at S1's invocation differs from the post-codegen IR dump**.  Need to instrument or read source.

### The corruption walk-through

If the second Set isn't S1-substituted, it's a regular `Set(cv, Call(alloc_canvas, [..., __ref_1]))`.  The pre-Set OpFreeRef on cv… but wait — the IR doesn't show a pre-Set FreeRef.  Did S2 fire?

Looking at probe 02's IR (which DOES leak), there's no pre-Set FreeRef either.  So pre-Set FreeRef apparently isn't emitted for these shapes — maybe because of the `depend().is_empty()` gate at `src/state/codegen.rs:1370`.  cv has dep on its own hidden attribute index, so `depend()` is non-empty, and the pre-Set FreeRef is skipped.

If that's true, then probe 04's mechanism is:

1. First Set (struct-lit): `OpDatabase(cv, 66)` — claims a NEW RECORD at cv's slot (with kind type 66 = Canvas).  Sets `data: 0` (null vec handle), `w: 1`.  cv's STORE remains the caller's hidden buffer, but the RECORD inside is fresh.
2. Second Set: alloc_canvas writes Canvas into `__ref_1`'s store (not cv's).  Returns __ref_1's DbRef.  Set assigns cv = __ref_1's DbRef.  **cv now aliases __ref_1's store — NOT the caller's hidden buffer.**
3. `OpFreeRefIfDistinct(__ref_1, cv)`: __ref_1 and cv now point to the same store; skip free.  __ref_1's store is preserved.
4. Return cv = DbRef into __ref_1's store.  Caller gets back a DbRef pointing to __ref_1, NOT the caller's pre-allocated buffer.

**Now the iter-2 corruption:** Main's caller-side flow:

- Iter 1: main calls `cv_c = render_lit_then_call(p)`.  render_lit_then_call's __ref_1 is in render_lit_then_call's frame, but its STORE (let's call it #X) is the actual data.  cv_c = DbRef #X.
- Iter end: cv_c goes out of scope (loop body iter-scoped).  OpFreeRef(cv_c) → frees #X.  __ref_1's frame is also gone.
- Iter 2: main calls again.  render_lit_then_call's NEW __ref_1 might get allocated at a different store, OR could reuse #X.
- Inside iter 2's call, the FIRST Set `OpDatabase(cv, 66)` claims cv's slot at caller's hidden buffer.  But cv's slot still holds the iter-1 DbRef (to #X, which is now freed).  When OpDatabase claims, it might either:
  - Allocate a fresh store for cv.
  - Reuse cv's existing slot_nr but for new content.

If reuse, iter 2's cv aliases iter-1's freed slot.  Then read of cv_c.data[0] reads from the freed slot, which still has iter-1 data.  That matches the symptom: `cv_c[0] = 1` at iter 2 (expected 2 = i; got 1 = i-1).

### Probe 04 vs reference 19 (wrap-in-struct)

Probe 19 also has a struct-literal in the body but the heap call is a FIELD-INIT, not a separate Set.  IR diff:

```
fn n_wrap(p:P) -> Wrapper {
  __lift_1(2):ref(Canvas) = null;
  __ref_2(1):ref(Canvas) = null;
  __ref_1(1):ref(Wrapper) = null;
  [27] {#Object(2):ref(Wrapper)["__ref_1"]
    OpDatabase(__ref_1(1), 66i32);              <-- Wrapper struct alloc
    __lift_1(2):ref(Canvas) = n_alloc_canvas(4, 5, p.tag, __ref_2(1));
                                                <-- Canvas alloc into __ref_2 (work-ref)
    OpCopyRecord(__lift_1(2), OpGetField(__ref_1(1), 0, 65), 65);
                                                <-- DEEP COPY Canvas into Wrapper.canvas
    OpSetInt(__ref_1(1), 12, OpGetInt(p(0), 0));   <-- set tag field
    OpFreeRef(__lift_1(2));
    OpFreeRef(__ref_2(1));
  ...
}
```

Probe 19 uses `OpCopyRecord` to deep-copy the heap call's result into the Wrapper's field.  Then explicitly frees both `__lift_1` and `__ref_2`.  No aliasing across iters.

Probe 04 has NO equivalent deep-copy or explicit free of the call's result — cv just aliases __ref_1's store, and the iter-end cleanup is the leakage source.

---

## Probe 28 — only-conditional-set (control-flow corruption)

```loft
fn render(p: P) -> Canvas {
  cv: Canvas = Canvas { data: [], w: 0 };  // Default-init
  if p.tag > 0 {
    cv = alloc_canvas(4, 5, p.tag);
  }
  cv
}
```

**Symptom:** At iter 2 (tag=2 > 0, condition IS true), the test reads `cv.w == 0` (the default) instead of `cv.w == 4` (the alloc_canvas result).  The conditional Set didn't propagate — even though the condition was true.

**Lowered IR**:

```
fn n_render(p:P, cv:Canvas) -> Canvas["cv"] {
  __ref_1(1):ref(Canvas) = null;
  [29] OpDatabase(cv(0), 65i32);              <-- DEFAULT-INIT into cv
  OpSetInt4(cv(0), 8i32, 0i32);
  OpSetInt(cv(0), 0i32, 0i32);
  [30] if OpLtInt(0i32, OpGetInt(p(0), 0i32)) {#block(2):void
    [31] cv(0):ref(Canvas) = n_alloc_canvas(4, 5, p.tag, __ref_1(1));
                                              <-- Conditional Set: writes via __ref_1
  } else null;
  [33] OpFreeRefIfDistinct(__ref_1(1), cv(0));
  return cv(0);
}
```

### Mechanism hypothesis

The default-init writes a Canvas record DIRECTLY into cv's (caller's hidden buffer) slot.  Then the conditional fires (when tag > 0), which:

1. Calls alloc_canvas, passing `__ref_1` as the hidden buffer.
2. alloc_canvas writes a fresh Canvas into __ref_1's store (NOT cv's).
3. Returns __ref_1's DbRef.
4. `cv = result` assigns cv to point to __ref_1's store.  **But the assignment lowers via the same path as probe 04 — and only updates cv's LOCAL DbRef, not the caller's slot.**

Wait — but cv IS the caller's hidden buffer.  Updating the local cv slot's DbRef to point to __ref_1 means the caller still expects cv's slot to be a Canvas record, but it now has a DbRef pointing elsewhere.

Actually thinking more carefully: `cv(0):ref(Canvas) = n_alloc_canvas(...)` — this is a Set that REPLACES cv's DbRef value with the call's return value.  The OLD content (default-init Canvas) is NOT in cv anymore from the callee's perspective.

But the CALLER's hidden buffer slot was where the default-init wrote.  Now the callee's cv variable points elsewhere.  When the callee returns, what does the caller see?

The buffer-passing contract: caller pre-allocates, callee writes INTO the buffer.  Caller READS from the buffer slot after the call.  If the callee updated its local cv to point elsewhere, the caller's slot still has the OLD content.

That's exactly probe 28's bug.  Iter 2 returns cv = __ref_1's DbRef from inside the call's frame, but main reads the buffer slot — which still has the default-init w=0.

### Why iter 0 and 1 work in probe 28

iter 0: tag=0, condition false.  No Set inside the if.  cv stays at default-init.  Main reads w=0.  ✅ matches expected.
iter 1: tag=1, condition true.  Set inside if fires.  Main… should read w=4.  Apparently does (assertion passes).
iter 2: tag=2, condition true.  Set inside if fires.  Main reads w=0.  ❌ FAIL.

The iter-1-works, iter-2-fails pattern suggests state accumulates.  Maybe iter 1's __ref_1 reuse pattern aligns with the caller's slot by luck (first-fit recycling), and iter 2 diverges because of state buildup.

Actually probe 28's symptom is similar to probe 04's: iter 2 first to fail, iter 1 might work via slot-recycling luck.  Both involve the caller-slot vs callee-local-cv discrepancy.

---

## Common thread between probes 04 and 28

Both shapes have:

1. A FIRST initialization of cv via struct-literal (direct field-init on cv's slot via OpDatabase + OpSetInt).
2. A SECOND assignment via Set with a Call RHS, where the Call uses a fresh `__ref_local` as its hidden buffer (S1 didn't substitute).
3. The Set updates cv's LOCAL DbRef to point to __ref_local's store, but the caller's hidden buffer slot still has the FIRST init's content.

The caller reads back the buffer slot, gets the FIRST content, not the SECOND.

This is a **callee-modifies-local-cv-but-caller-reads-original-slot** problem.  Probe 01 doesn't have it because there's only ONE Set, which is S1-substituted (call writes into cv's slot directly).  Probes 04 and 28 have a struct-literal that initializes cv's slot in-place, followed by a Set that DIVERTS cv to a different store via Call.

## What native does correctly

Both probes 04 and 28 pass on native.  Native's codegen for `Set(cv, expr)` where cv is a hidden buffer probably either:

- Returns the new value through the cv parameter binding (Rust borrow semantics).
- Updates the buffer in-place by writing field-by-field.

The native ABI handles this naturally because Rust's `&mut` reference automatically reflects callee modifications in the caller's slot.

## What we know vs. don't

| | Status |
|---|---|
| IR shapes for 04 and 28 | ✅ Dumped (`/tmp/bc_04.txt`, `/tmp/bc_28.txt`) |
| Both have struct-lit + call sequence | ✅ Verified |
| Both have S1 not substituting the call | ✅ Visible in IR (call uses `__ref_local`, not `cv`) |
| The caller-slot-vs-callee-local divergence | 🤔 Hypothesized; consistent with symptom (caller reads default) |
| Why iter 1 works but iter 2 fails | 🤔 Probably first-fit luck on iter 1; iter 2 has more state buildup |
| Why S1 doesn't fire here when its preconditions seem to match | ❌ Unknown.  l's layout at S1's invocation needs inspection |

## Investigation tasks

1. **Instrument S1 with eprintln** to verify whether `nrvo_collapse_tail_set` is called on probe 04's `render_lit_then_call` and what l's structure is.
2. **Read the Set-Reference codegen** at `src/state/codegen.rs:1367-2050` — specifically how `Set(cv, Call(fn, args))` is lowered when cv is a hidden buffer.  Is the result DbRef written back to cv's slot, or does it just update cv's local register?
3. **Read native's Set-Reference codegen** for comparison.
4. **Check the Object/struct-literal codegen** — confirm `cv = Canvas { ... }` lowers to direct field-set on cv's slot (in-place), not to a fresh store + DbRef update.

## Fix surface

**(a) Make the Set always write through the buffer slot.**  When `Set(cv, Call)` where cv is a hidden buffer, ensure the result is copied INTO cv's slot (not just replacing cv's local DbRef).  This is the buffer-protocol contract.  Effort: M — needs the codegen to detect the buffer case and emit OpCopyRecord into cv after the call returns.

**(b) Extend S1 to fire on these shapes too.**  If S1 substitutes, the call writes into cv directly and there's no caller-slot-vs-local-cv divergence.  But the preconditions need broadening.  Effort: M+ (cumulative; each shape adds preconditions).

**(c) Path C — store refcount.**  Same as Cluster II's analysis.  Refcount-based ownership eliminates the manual-buffer-slot model entirely.  Effort: L; subsumes II + III + probably IV-IDX too.

**Most likely best path: (c)** for the same reason as Cluster II — the underlying model is fundamentally about manual-buffer-slot semantics not handling all shapes.  Patching shape-by-shape is whack-a-mole.
