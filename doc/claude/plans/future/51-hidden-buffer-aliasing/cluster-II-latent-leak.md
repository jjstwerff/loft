# Cluster II — Latent leak (interpret-only)

**Status (2026-05-29): 🟢 LARGELY CLOSED — Step 2 + post-call free+reset landed.  Probes 02, 03, 07, 11, 21, 25, 26 are leak-free.  Probes 04 + 28 still leak Canvas×5 (struct-literal-first / default-init-first patterns — caller_hidden_args reset doesn't apply).**

### Step 2 + post-call free + reset (2026-05-29)

Landed in `src/state/codegen.rs` at both `gen_set_first_ref_call_copy` (line 2060+) and the reassignment path (line 1472+):

1. **Step 2 — refined `is_borrowed_view`**: require at least one VISIBLE-arg dep before treating the return as a borrowed view.  Hidden-only deps (ref_return-promoted buffer attrs) now enable `0x8000` source-free.
2. **Post-call free + reset sequence**: for each caller-hidden-buf arg in the call (filtered to exclude the LHS), emit:
   ```
   OpVarRef(slot) → OpFreeRef → OpInitRefSentinel(slot)
   ```
   `OpFreeRef` (via `free_named`) is idempotent — it handles sentinel store_nr (no-op) and already-freed stores (no-op).  This makes the sequence safe whether OpCopyRecord's `0x8000` already freed the source OR @P290's `protect_store_frees` blocked it (the placeholder's store is then still live).  The sentinel write stops the next iter's `OpDatabase` from reclaiming a recycled store_nr that the allocator may have handed to a different var.

3. **Bytecode VM OpDatabase sentinel handling** (`src/state/io.rs:723`): when the slot DbRef holds `store_nr == u16::MAX`, allocate a fresh store via `null()` and write the new DbRef back to the slot.  Mirrors the native runtime's `OpDatabase` semantics.

### Validation

- `cargo test --release --test leak_cases leak_cases_interp` — passes (was previously regressing `clean/local_var_return_shifted_var_nr` until the free was added to the reset sequence; that test exercises the @P290-protected placeholder shape where the source store is NOT freed by OpCopyRecord and the codegen MUST emit its own free).
- 50 V-class probe runs (--interpret + --native) — no regressions.

### Remaining work

Probes 04 (struct-literal first Set, then Call Set) and 28 (default-init first Set inside `cv: Canvas = ...`, then conditional Call Set) still leak Canvas×5 each iter.  Their hidden-buf placeholder is NOT a `caller_hidden_buf`-marked work-ref at the second Set site (the first non-Call Set initialised cv from a struct-literal RHS path, which doesn't go through the call-copy codegen).  Fix likely requires extending S1 to recognise struct-literal-first or default-init-first patterns, OR a parser-side marker that flags every reassignment site downstream of a `caller_hidden_buf` arg.

### Historical record (pre-2026-05-29)

**Status (2026-05-28): 🟡 PARTIAL — probes 02 + 21 closed, 5 still leak.**

Commits:
- `d710e399` (Cluster III fix) — incidental: also closes the corruption variants in probes 04 / 28; their assertions now produce correct values, only the leak warning remains.
- `ff0b38d4` (Cluster II partial) — extends `nrvo_collapse_tail_set` backwards through consecutive `Set(cv, Call(_))` ops.  Closes probes 02 and 21 (the canonical double-Set shapes) AND eliminates the per-iter leak entirely for those.

**Still leaking (all `--interpret`, assertions PASS, only the warning):**
- Probe 03 (intervening stmt) — non-Set op breaks the consecutive walk.
- Probe 04 (struct-lit + call) — first op is a struct literal, not a Call.
- Probe 07 (explicit return) — `parse_return` doesn't invoke `nrvo_collapse_tail_set`.
- Probes 11, 25, 26 (conditional Set in If branch) — If wrapper breaks the walk.
- Probe 28 (default-init + conditional Set) — same If issue.

An attempt to extend the substitution to ALL `Set(cv, Call(_))` in the body (including descent into If/Block/Return wrappers) regressed `tests/scripts/87-store-leaks.loft` (NaN result).  Rolled back to the consecutive-Set-only version.  Future work: branch-aware substitution that respects probe 87's invariants, OR a runtime-side fix at `OpFreeRefIfDistinct` to detect reassignment-after-adoption.

### Failed approach #2 — caller-side `is_borrowed_view` refinement (2026-05-28)

Tried refining `is_borrowed_view` at `src/state/codegen.rs:1479` and `:2010` to require at least one VISIBLE arg dep, so hidden-only deps would enable 0x8000 source-free.  Theory: for hidden-only deps, source is either (a) the caller's hidden buffer (same-store guard skips free) or (b) a fresh S1 (0x8000 safely frees).

Outcome: closed probes 03, 04, 07, 11, 25, 26 but REGRESSED probes 02, 21, 28 (assertion failures, data corruption).

Root cause: with extended S1 (commit `ff0b38d4`), the canonical multi-Set callee returns the caller's hidden buffer.  The OpCopyRecord's src and dst share a store.  The runtime guard at `src/state/io.rs:1224-1232` skips the `free` call — but does NOT skip the `remove_claims` prelude at line 1208, which frees nested vec records BEFORE `copy_block` reads them.  Same-store OpCopyRecord with nested heap fields is fundamentally broken; the existing `has_hidden_ref` check was protecting against this.

Reverted to baseline.  Future fix must EITHER:
1. Make OpCopyRecord's same-store path a no-op (skip remove_claims + copy_block + copy_claims when data.store_nr == to.store_nr).
2. Avoid generating same-store OpCopyRecord at codegen time (track which Call results would alias the destination).
3. Track ref ownership at runtime via refcounts (the `project_drop_store_refcount` arc).

### Defensive hardening landed (2026-05-28, commit `172171ee`)

Path (1) above shipped as a standalone defensive fix: both backend `OpCopyRecord` implementations (`src/state/io.rs::copy_record` line 1196+ and `src/codegen_runtime.rs::OpCopyRecord` line 490+) now early-return when `data == to` (full DbRef equality).  This closes the destructive-prelude window the previous failed caller-side fix exposed.

**However**, a re-attempt of the caller-side `is_borrowed_view` refinement ON TOP of this hardening still regresses probes 02, 21, 28 with the same `cv_a.data[0] = w_value` corruption — so the corruption is NOT primarily caused by same-store OpCopyRecord.

### True mechanism pinned via bytecode runtime tracing (2026-05-29)

Added `LOFT_TRACE_DB` (OpDatabase calls) + `LOFT_TRACE_CR` (OpCopyRecord src/dst + vec contents) tracers — both env-var-gated, left in tree.

Probe 02 trace with Step 2 active reveals the cross-iter slot dangling:

```
iter 0:
  OpDatabase var=24 (p)    db=#3@0,8       → p at #3
  OpDatabase var=12 (cv_a) db=#4@0,8       → cv_a at #4
  OpDatabase var=44 (mlb)  db=#2@0,8       → main_local_buffer (mlb) at #2
  OpDatabase var=44 (mlb)  db=#2@1,8       → second alloc_canvas reuses #2
  OpCopyRecord src=#2(w=4,data_ptr=13,vec0=1) dst=#4 free_src=true
                                            → main's wrap: copy + FREE #2
  scope-exit frees #4 (cv_a), #3 (p)

iter 1:
  OpDatabase var=24 (p)    db=#2@0,8       → p NOW at #2 (slot reuse after free)
  OpDatabase var=12 (cv_a) db=#3@0,8       → cv_a at #3
  OpDatabase var=44 (mlb)  db=#2@0,8       → mlb's slot still has stale #2!
                                            → clear(#2) WIPES p's data
                                            → claim returns rec=1, mlb := #2@1,8
                                            → mlb and p now alias the SAME RECORD
  ... render_double's first alloc_canvas writes w=3 into "p's" record
  ... second alloc_canvas overwrites with w=4
  OpCopyRecord src=#2(w=4,data_ptr=13,vec0=4) ...
                                            → vec0=4 because p's data got
                                              clobbered by the canvas record
                                              that aliased it
```

**The cross-iter dangling slot is the root cause.**  My Step 2's `0x8000` source-free at main's wrap frees `main_local_buffer`'s store, but doesn't reset `main_local_buffer`'s slot DbRef.  Next iter, the allocator reuses that store_nr for a different var (`p`); `main_local_buffer`'s slot still points there.  When render_double's `OpDatabase(mlb)` runs, it `clear()`s the store — wiping `p`'s data — and then both slots alias the same record.

### What's needed for a real fix

Source-free of a caller-LOCAL slot's contents requires resetting that caller's slot to a sentinel after the free.  The OpCopyRecord runtime doesn't know which var holds the source DbRef.  Either:

1. **Codegen-side post-call sentinel reset**: in `gen_set_first_ref_call_copy` and the reassignment path, identify the call's hidden-buf args (caller_hidden_buf-marked work-refs) and emit `Set(arg_var, Null)` after the OpCopyRecord wrap.  Adds ops per call.
2. **Inhibit source-free for caller_hidden_buf args**: detect when the call's hidden-buf arg IS the same DbRef the callee might return (which is the canonical S1 case), and skip source-free.  Detection is tricky without flow analysis.
3. **Reset slot from runtime**: extend the OpCopyRecord protocol with a "source var index" so the runtime can also reset that slot on free.  Bytecode protocol change.
4. **Refcount-based ownership** (`project_drop_store_refcount`): subsumes all of this.  L-effort architectural change.

Status: probes 02, 21 stay leak-free (extended S1 commit `ff0b38d4`); probes 03, 04, 07, 11, 25, 26, 28 still leak.  No silent corruption anywhere — all assertions PASS.

Step 2 NOT shipped (would require parallel fix above).  Future investigators have:
- `LOFT_TRACE_CR=1` — every OpCopyRecord with src/dst + Canvas field contents before/after.
- `LOFT_TRACE_DB=1` — every OpDatabase call with var/type/DbRef.
- `LOFT_TRACE_FINISH=1` — every finish_type entry/exit for tuple types.
- `LOFT_TRACE_COPY=1` — native-side OpCopyRecord trace.

---

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
| Slot allocation is CLEAN for all 6 Cluster II probes (02, 03, 07, 11, 25, 26) | ✅ Verified via `LOFT_LOG=slots:n_render` — only is_argument SKIPs for params, no unallocated vars |
| **The mechanism is RUNTIME, not parse/codegen** | ✅ Slot trace clean → bug is in opcode execution at runtime, consistent with child-store-orphan hypothesis |
| The child-store-orphan mechanism | 🟢 Hypothesized; consistent with the +1 per iter store trace; probe 36 confirms per-iter-not-per-Set scaling |
| The exact opcode that overwrites without recursive-free | ✅ **Verified — `OpDatabase` reuse-path calls `stores.clear()` which only resets store metadata; no recursive walk of Reference-typed fields** |
| Why probe 07 (explicit-return) leaks identically | 🤔 ref_return doesn't fire on Return-tail bodies; needs source reading at `parse_return` |
| Why probe 26 leaks when if-false never fires | 🤔 The conditional Set's CODEGEN affects buffer-protocol setup; the if's then-block contains Set(cv, ...) whose presence alone perturbs slot tracking |

## Investigation results (2026-05-28, code-only Plan agent)

A code-only investigation agent walked the relevant source paths.  Key file:line findings:

| File | Lines | What it does | Why it's relevant |
|---|---|---|---|
| `src/parser/objects.rs` | 1538-1544 | Struct-literal lowering for `cv = Canvas{…}` when LHS is an existing slot var.  Guards `v_set(Null)` prelude on `!is_argument(v_nr)`.  Emits `OpDatabase(cv, tp)` directly. | For hidden-buffer params (`is_argument == true`), the null-prelude is SKIPPED.  OpDatabase fires on the existing slot. |
| `src/codegen_runtime.rs` | 210-230 | `OpDatabase` runtime.  Branches on `db.store_nr == u16::MAX` (fresh alloc) vs. reuse.  In reuse path: calls `stores.clear(&db)` then `claim`. | The reuse path is where the leak happens — `clear()` does not free child stores reachable through old record's Reference-typed fields. |
| `src/database/allocation.rs` | 420-430 | `Stores::clear()` — calls `store.init()`. | `init()` resets the store's allocator metadata (free-list / claims) but does NOT walk the type's Reference fields to free externally-owned child stores. |
| `src/database/allocation.rs` | 166-295 | `Stores::free_named` — cascade-free walk for child stores.  But the cascade gate at 206-238 **only fires for `__closure_*`-prefixed types**. | Plain user types (Canvas) get NO cascade.  This is why scope-exit `OpFreeRef` on `cv` doesn't free its child vector store either — same gate. |
| `src/scopes.rs` | 681 | `paired_witness` map insertion uses `entry().or_insert`. | If multiple `__ref_N → witness_v` pairings exist for the same witness (probe 36's 3-Set case), only the FIRST gets recorded.  Subsequent pairings stale → pair-free check fires wrong. |

### Refined mechanism

The original cluster doc hypothesis (V_1 = separate vector child store) was partially imprecise.  `vector_append` actually claims a child **record** within the parent store, not a separate store.  Closer reading:

**For probe 02 (single non-S1 Set + one S1 Set):**

1. **First call** passes `__ref_1 = null` to alloc_canvas.  Inside, `cv = Canvas { data: [], w: 3 }` invokes OpDatabase with `cv.store_nr == u16::MAX`, allocating store `S_a`.  `cv.data += [fill]` claims a child record in `S_a` for the vector.  Returns `DbRef{S_a, …}`.

2. **Set assigns cv = __ref_1's DbRef.**  cv aliases `S_a`.

3. **Second call** (S1-substituted, buffer = cv = `DbRef{S_a}`): alloc_canvas's `cv = Canvas { data: [], w: 4 }` invokes OpDatabase with `cv.store_nr == S_a` (reuse path).  `clear()` calls `store.init()` — wipes `S_a`'s allocator metadata.  Then `claim` allocates a fresh Canvas record in `S_a`, and `cv.data += [fill]` claims a fresh vector child record.  The OLD record's bytes are still in `S_a` but the allocator considers them free.

4. **Scope exit:**
   - `OpFreeRefIfDistinct(__ref_1, cv)`: both `DbRef{S_a, …}` → skip.  `S_a` is the caller's buffer; caller will free it.
   - `OpFreeRef(__ref_2)`: __ref_2 is null → no-op.

5. **Caller (main) end-of-iter:** frees `S_a`.

**Where is the per-iter leak then?**  Per the agent's analysis, the leak source isn't reconstructable from the code alone — the per-iter +1 store-count growth needs runtime store-trace analysis to pin which exact store doesn't get freed.

One leading hypothesis from the agent: the `paired_witness::entry().or_insert` issue at `src/scopes.rs:681` may leave `__ref_1` un-pair-freed in multi-Set scenarios, while `__ref_2` (and others) is.  In probe 02, the IR shows only ONE `OpFreeRefIfDistinct(__ref_1, cv)` at scope exit — there's no corresponding free for `__ref_2`'s store (which would be a fresh alloc inside the first alloc_canvas call's internal `__ref_1`, NOT render_double's `__ref_1`).

**The mechanism is partially verified — the orphan opcode site is unclear without a runtime store-trace pinpointing the leaked store_nr.**

## Proposed fix (M effort)

The investigation agent proposed two-part fix:

### Part 1 — Recursive pre-clear free in `OpDatabase` reuse path

`src/codegen_runtime.rs` OpDatabase (lines 210-230).  Before `stores.clear(&db)` reinitialises a non-null store, walk every Reference / Vector / Enum child store held by the OLD record and free them.  Pseudo-Rust:

```rust
pub fn OpDatabase(cell, mut db: DbRef, db_tp: i32) -> DbRef {
    let stores = unsafe { &mut *cell.get() };
    let db_tp = db_tp as u16;
    let size = stores.size(db_tp);
    if db.store_nr == u16::MAX {
        db = stores.null();
    } else {
        // NEW: reusing a populated slot.  Walk old record's Struct fields
        // and free DbRef-holding child stores before clear() wipes metadata.
        let old_kt = stores.allocations[db.store_nr as usize].known_type;
        if old_kt != u16::MAX && db.rec != 0 {
            if let Parts::Struct(fields) = &stores.types[old_kt as usize].parts.clone() {
                for f in fields {
                    if matches!(stores.types[f.content as usize].parts, Parts::DbRef) {
                        let off = db.pos + u32::from(f.position);
                        let cs = stores.allocations[db.store_nr as usize]
                            .get_u32_raw(db.rec, off) as u16;
                        let cr = stores.allocations[db.store_nr as usize]
                            .get_u32_raw(db.rec, off + 4);
                        if cs != u16::MAX && cs != db.store_nr && cr != 0 {
                            stores.free(&DbRef { store_nr: cs, rec: cr, pos: 8 });
                        }
                    }
                    // Inline Vector/keyed fields share the parent store; clear() reclaims them.
                }
            }
        }
    }
    stores.clear(&db);
    let r = stores.claim(&db, u32::from(size));
    ...
}
```

### Part 2 — Symmetric extension of `free_named`'s cascade gate

`src/database/allocation.rs:206-238`.  Extend the `__closure_*` cascade to include plain struct types with Reference-typed fields when those fields point to **distinct stores** (`cs != parent.store_nr`) AND `ref_count <= 1`.  Unifies the recursive-free path so scope-exit OpFreeRef on owned refs also cascades.

### Why this works

Matches native's implicit "old value drops first" semantics — the cluster doc's note that native is clean confirms native's codegen does this via Rust drop.  The fix retrofits the same semantics into the interpret runtime.

### Effort and risk

**Effort: M (1-3 days)** — runtime change touches OpDatabase + free_named on both backends (`src/codegen_runtime.rs` for native runtime + `src/state/io.rs` for the bytecode-VM OpDatabase handler).

**Risks:**
- **Over-free of aliased child stores.** Deep-slice borrows (probe 39) share child stores between parent records.  Mitigation: only cascade when `cs != db.store_nr` AND the child's `ref_count <= 1`.  Existing rc-aware free at allocation.rs:179-189 already handles this.
- **`Parts::Array(_)` fields** (per @P376) — vector fields may store linked element records.  Need to verify whether array elements are in-store or cross-store; probably in-store (claim in parent's store at allocation.rs:109).
- **Interaction with `paired_witness` / `OpFreeRefIfDistinct`.**  If the cascade frees a child store that's also a witness in a pair-free, the subsequent `OpFreeRefIfDistinct` would no-op (double-free guard at allocation.rs:175).  Safe.

### Files to change

1. `src/codegen_runtime.rs` — `OpDatabase` (lines 210-230): add recursive pre-clear free.
2. `src/database/allocation.rs` — `free_named` (lines 206-238): extend cascade gate beyond `__closure_*`.
3. `src/state/io.rs` — bytecode-VM `OpDatabase` handler (around line 727): mirror the runtime fix.
4. (Optional alternative location) `src/parser/objects.rs` (lines 1538-1544): emit a pre-`OpDatabase` cascade-free opcode at parse time instead of runtime.  Runtime fix is cleaner — one location.

### Does this close probe 39 (moros_map leak)?

**No.**  Probe 39's mechanism is structurally different:
- Pattern: `return gh_c.ck_hexes[idx]` — a deep-slice borrow return out of a nested struct field.
- The returned value SHARES a store with the outer parameter `m`; the leak (if real) is in how the deep-slice's dep-chain interacts with the hidden buffer.
- The fix needed is in the deep-slice / iterator-tail return codegen — separate sub-mechanism, likely Cluster III territory.

Probe 39 should be addressed separately after Cluster II's fix lands.

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
