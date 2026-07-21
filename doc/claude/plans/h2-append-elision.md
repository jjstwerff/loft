# H2 — `vec += [f(temp)]` in a loop drops every element but the first

> **Status: FIXED — root cause was a missing runtime ownership guard on FIRST
> assignment; see § THE FIX.** One arm in `src/state/codegen.rs`. Interpreter only;
> `--native` was already correct, so it was also a backend divergence. Silent — exit 0,
> no diagnostic, wrong data. Reported by the crawler consumer (`LOFT-HANDOFF` H2) on
> toolchain 2026.7.2 as the wider form of **loft#496**, which is CLOSED — so either that
> fix was too narrow or this is a regression of it. Probes: `h2-append-elision/probes/` —
> THREE matrices, `run.sh` (call-site axis), `run-callee.sh` (callee-return axis) and
> `run-join.sh` (first-Set-vs-reassign axis, the one that found the cause); run all three.

## RESUME — start here

**One-line state:** H2 is fixed at its real root cause and every cell of all three
matrices is green except two INDEPENDENT, pre-existing leaks (`j6`, `j8`-native) that the
fix neither causes nor addresses. The `value-before-slot.patch` is **abandoned** — it
treated a symptom, and with the real fix in place none of its four regressions exist.

**Validate with all three matrices, never one:**

```bash
P=./doc/claude/plans/h2-append-elision/probes
$P/run.sh          # call-site axis      (p1-p8)   → expect 0 differing
$P/run-callee.sh   # callee-return axis  (r1-r12)  → expect 2 (r5/r6, § Still open)
$P/run-join.sh     # first-Set axis      (j1-j8)   → expect 3 (j6 ×2, j8-native)
```

Each harness prints its own baseline and warns that a 0 may mean the instrument went
blind. All three assert value AND length AND leak on BOTH backends.

**Still open, all independent of H2** — none reproduces without a `??` join, none is
caused by this fix, and **each needs checking against `main` before filing**, per the
bug-filing policy:

| cell | backends | shape | note |
|---|---|---|---|
| `j6_base_not_var` | **both** | `pick(b.items, i)` — the join's borrow base reaches the callee as a FIELD expression | no local names the base, so `ownership_of` answers `base == u16::MAX` and neither backend can witness the guard. A structural blind spot, not a missed case. |
| `j8_reassign_join` | native | the store displaced by the FIRST reassignment | the interpreter frees it; native does not — the exact mirror of what this plan previously assumed the polarity to be |
| `r5`/`r6` | interpret | callee returns its retbuf on one path, another call's store on the other | 2 stores, no loop needed |

**Branch:** `tuxedo-win-oracle`, stacked on PR #609, **no PR of its own**.

**Environmental suite failures here** (not regressions): `index_hygiene_clean` (a link to
the git-ignored, locally-built `doc/claude/LIBRARIES.md` — run `make libcatalogue`) and
`wasm_debug_relay`.

## THE FIX — a first assignment never got the runtime ownership guard

`OpBindOrCopy` (@PLN85) is the interpreter's per-execution adopt-vs-copy guard: given the
call result and a *witness* (the variable the return may borrow), it adopts the store when
the two differ (the callee minted it) and deep-copies when they match (the callee handed
back a view). It was emitted **only on the reassignment path**. A FIRST assignment chose
adopt-vs-copy statically from `returns_borrowed_view()`.

For a callee whose return is a runtime join — `fn pick(t, i) -> M { t[i] ?? m_none() }` —
no static answer is right for both arms. The verdict came back "borrowed view", so
`gen_set_first_ref_call_copy` suppressed the `0x8000` source-free, and **every execution
that took the fresh arm orphaned the store the callee had just minted.**

A loop body re-declares its local every iteration, so the binding is a first Set every
time and the leak scales with the trip count. That is the whole four-way boundary below:
no loop → one orphan instead of N; a literal temp → no call to join; an intermediate local
→ a distinct store; no temp → nothing bound.

**The orphaned stores are also what corrupted the values.** Freed-then-recycled store slots
were handed back out and written through while a vector element still pointed at them — so
fixing the leak fixed H2's `1 null null` directly. The corruption and the leak were one
defect, not two.

**Fix:** `src/state/codegen.rs::gen_set_first_ref_join` — when `ownership_of` classifies
the call's return as `Own::Join { base }` with a resolvable base, bind through
`OpBindOrCopy` exactly as the reassignment path does. `v`'s ordinary scope-exit `OpFreeRef`
is then correct on both arms. One subtlety worth keeping: the slot preamble must use
`OpInitRefSentinel`, not `OpInitRef` — the latter writes `Stores::null()`, which
*allocates* an empty store, and the adopt arm overwrites the slot, so `OpInitRef` traded
the old leak for a new one of the same size (measured: `M×3` became `?×2, M×1`).

### Why the earlier "value-before-slot" fix looked right and was not

That patch reordered the append so the call ran before `OpNewRecord` took a `DbRef` into
the vector's store. It turned all 8 `p`-cells green because it removed the *window* in
which a recycled store could clobber the element — but the recycling itself, the actual
defect, was untouched. Hence its four suite regressions and the `r10` leak it "introduced":
it was inheriting the orphan, not creating it. With the guard in place the reordering buys
nothing, so the patch is abandoned and the two `AppendSource` expectations and the
fuzz-gate control need no re-basing.

## Symptom

```loft
for i in 0..3 { d = pick(t, i); out += [mk(d)]; }
```

Every appended element after the first reads back with **all fields null** — the whole
record, not just its text fields. A store leak accompanies it
(`1 stores not freed at program exit`).

## The boundary — it needs FOUR things at once

Measured on both backends (`probes/`, hand-computed expectations):

| probe | shape | interpret | native |
|---|---|---|---|
| `p1_callres_loop` | the reported shape | **`1 null null`** | `1 2 3` |
| `p5_twoargs` | same, extra scalar arg | **`1 null null`** | `1 2 3` |
| `p2_single` | one iteration, no loop | `1` | `1` |
| `p3_straight` | two appends straight-line, temp reassigned | `1 2` | `1 2` |
| `p4_literal_tmp` | temp from a LITERAL, not a call | `1 2 3` | `1 2 3` |
| `p6_via_local` | `e = mk(d); out += [e]` | `1 2 3` | `1 2 3` |
| `p7_append_tmp` | `out += [d]` — append the temp itself | `1 2 3` | `1 2 3` |
| `p8_no_tmp` | `out += [mk(pick(t,i))]` — no temp at all | `1 2 3` | `1 2 3` |

So it requires **all four** of: a **loop** (`p3` straight-line is fine) · the temp
assigned from a **call** (`p4` literal is fine) · passed **by value into another call**
(`p7` appending the temp is fine) · whose result is appended **directly**
(`p6` via a local is fine).

`p3` passing while `p1` fails is the key cell: the difference is the loop's
per-iteration scope exit, not the reassignment.

## Localised (superseded — the copy/adopt difference is real, the reading of it was not)

`loft introspect` on the failing shape beside its working sibling (`p1` vs `p6`),
loop body only:

| | `p6` (works) | `p1` (fails) |
|---|---|---|
| after the `mk` call | **`CopyRecord(data, to, tp=66)`** | *absent* |
| `FreeRef` in the loop body | 1 | **2** |

The via-local path **copies** the returned record into the vector element. The direct
append instead **adopts** the callee's returned store — and the loop's scope exit
still frees it, so every element but the last-appended points at freed memory. The
extra `FreeRef` is that free; the leak warning is its mirror image.

## Theory 3, FALSIFIED — the NRVO return buffer freed once per iteration

The first theory (a double free from the copy's `0x8000` source-free bit plus the
lift's scope free) was **tested and wrong**: clearing the bit leaves the IR with a
single free and the bug unchanged. Reverted.

The IR names the real shape:

```
fn n_mk(d: D, __retbuf: E) -> E          // the result is written into a CALLER buffer
185[136]: Database(var[112], db_tp=65)   // that buffer is allocated ONCE, outside the loop
  __lift_1 = n_mk(d, __ref_3);           // __lift_1 ALIASES the buffer
  OpCopyRecord(__lift_1, _elm_1, 66);
  OpFreeRef(__lift_1);                   // ... and frees it EVERY iteration
```

`scopes.rs`'s `scan_args` lifts the call result into `__lift_N` so scope exit frees
it — correct when the temp owns a fresh store, wrong here, because the store is the
enclosing scope's NRVO return buffer. After iteration 1 the buffer is freed; later
iterations write into freed memory, which by then has been reallocated as a vector
element record — so each iteration corrupts what the previous one appended. That is
exactly why every element **but the last-appended** reads null.

It also explains the four-way boundary: no loop → no reuse (`p3`); a literal temp →
no NRVO call (`p4`); an intermediate local → the result lands in a distinct store
(`p6`); no temp → nothing to lift (`p8`).

**Corrected invariant:** a `__lift_N` temp may free its store only if it OWNS it. A
lift that aliases a caller-provided return buffer must not emit a free — the buffer's
owner is the enclosing scope.

**Fix site:** `scopes.rs` — the lift must recognise an NRVO-buffer-aliasing result and
skip the scope free (or bind without taking ownership). Neither fix option in the
previous section survives: copying more does not help, because the corruption is the
buffer free, not the copy.

## FALSIFIED — the per-iteration free is NOT the defect

The § Refined theory above (theory 3) said the bug is the per-iteration free of the
NRVO buffer. **The lifetimes-tool upgrade below falsifies it.** The check flags exactly
that shape, and it fires on **four** cells — `p1` and `p5` (broken) but also
`p4_literal_tmp` and `p8_no_tmp`, which **produce `1 2 3` on both backends**. Identical
IR shape, opposite outcomes, so the free alone cannot be the cause.

Captured beside its working sibling (`probes/`, loop bodies only), the real
discriminator is visible in one line:

| | free emitted at scope exit |
|---|---|
| `p6_via_local` (works) | `OpFreeRefIfDistinct(e, __ref_3)` — **guarded** |
| `p1`/`p4`/`p5`/`p8` | `OpFreeRef(__lift_N)` — **bare** |

So the guard mechanism **already exists** and the named-local spelling gets it; the
compiler-generated `__lift_N` path does not. `p4`/`p8` share the bare free and are
**latent** — nothing recycles the freed slot there — which is why they pass today.

**Corrected invariant:** a binding that may alias a caller-provided `__retbuf` must
free through the distinct-guard, never unconditionally. `p6` is the proof it is the
intended protocol.

**Not the fix site (measured, both reverted):** the two witness-pairing arms in
`scopes.rs::scan_set` (@P378(a) `witness_buffer`, ~:2846, and the @PLN85
`paired_witness` arm, ~:2918). Routing lifts into either moved **no IR at all** —
both sit inside `if adopts_fresh_store`, which is false for this callee, and the
@PLN85 arm additionally requires `function.is_argument(ov)` while `__lift_N` is a
local. The emission site for a lift's scope-exit free is upstream of both and is where
the next session should start.

## Theory 4, SUPERSEDED — the element slot is reserved BEFORE the value is computed

With the free ruled out (§ FALSIFIED), the last difference between the broken form and
its working sibling is **ordering**:

```
p1 (fails):  PreAllocVector; _elm_1 = OpNewRecord(out,…);  __lift_1 = n_mk(…);  Copy → _elm_1
p6 (works):  e = n_mk(…);  PreAllocVector;  _elm_1 = OpNewRecord(out,…);        Copy → _elm_1
```

`OpNewRecord` hands back `_elm_1`, a `DbRef` **into the vector's store**. The direct-append
form runs the call *after* taking that reference, so the callee's allocations recycle
stores behind it and the copy writes through a slot that is no longer the element. The
named-local spelling evaluates the call as its own statement and so has always emitted
the safe order. **Value first, then slot.**

Proven: hoisting the call into a temp before `OpNewRecord` turns **all 8 cells green on
both backends**, with no leak — and the resulting IR is identical to `p6`'s, including
`OpFreeRefIfDistinct`, which the EXISTING @P378(a) machinery supplies for free once the
source is a real local. No new special case.

### Why it was not landable — blast radius (moot; see § THE FIX)

Kept as `h2-append-elision/value-before-slot.patch` (not committed to `src/`). Against
the full suite it regresses **4** tests that pass without it:

| test | why |
|---|---|
| `join_own_fixes_elem_accumulate_both_backends` | **a real leak I introduce** — `1 stores not freed: M×18`. The hoisted temp is REASSIGNED each iteration; when the callee returns a FRESH store (not the retbuf) the displaced store is never freed. |
| `ownership_surfaces_free_sites`, `ownership_resolves_the_borrow_base` | the `AppendSource` ownership classification keys on the append source BEING a call; hoisting changes that shape |
| `fuzz_gate_positive_control_pairs` | *"harness is VACUOUS (crash channel fires=False)"* — the fix repairs the very crash the gate uses as its positive control, so the control must be re-based |

Narrowing the hoist to calls handed a `__ref_N`/`__rref_N` work-ref (the only shape whose
result can alias a reused buffer) cleared one regression but not these four.

**The blocking item is the leak, not the pinned shapes.** The hoisted temp must
participate in displaced-owned-store freeing on reassignment — i.e. reuse the existing
lift/`join_own` machinery rather than a bare `create_unique` temp. Do that first; the two
`AppendSource` expectations and the fuzz-gate control are then legitimate re-bases, but
they must NOT be edited before the leak is closed, since they are what caught it.

## The callee-return matrix (`probes/run-callee.sh`) — the axis the p-cells could not see

The `p1`–`p8` corpus varies the CALL-SITE shape and holds the callee fixed (every cell
calls the same struct-literal `mk`).  That is the wrong axis for validating this fix,
whose correctness depends on WHAT STORE the callee returns — which is why the patch read
green on all 8 p-cells while leaking, and the leak only surfaced 224 s later in the full
suite.  `r1`–`r12` vary the callee and hold the call site fixed; see `probes/README.md`.

Two results it produced immediately:

1. **The blocker is now a 3-second cell.** `r10_orelse_fresh_loop` passes on a clean tree
   and leaks `M×3` with the patch — the same defect the suite reported as `M×18`. The
   decisive kind is a callee that returns **borrow OR fresh, decided at runtime**
   (`t[i] ?? m_none()`): the `r5`/`r6` "retbuf or other-call" branch does NOT reproduce
   it. A fix for the hoist is done only when `r10` is green and `r1` stays green.
2. **An independent pre-existing leak.** `r5`/`r6` leak 2 stores on the CLEAN tree,
   interpreter only, with correct values — and need no loop, so it is not H2. The patch
   neither causes nor fixes it. **Not yet checked against `main`**; do that before filing.

Also worth recording: `r1`–`r6` all carry the same static verdict (`Owned` from
`--show-ownership`) yet behave differently, so the ownership classification does not
separate this axis — only running the cells does.

## The hoist is NOT the cause of the r10 leak — it inherits a pre-existing one

Measured, and it overturns the "blast radius" reading in the section above:

| shape | clean tree | with the patch |
|---|---|---|
| `out += [pick(t, i+100)]` (direct append, `r10`) | no leak | **leaks `M×3`** |
| `e = pick(t, i+100); out += [e]` (named local) | **leaks `M×3`** | leaks `M×3` (patch does not touch it — its append source is a `Var`, so the hoist never fires) |

So the named-local form ALREADY leaks on `main`-as-branched, and the hoist simply makes
the direct-append form emit that same IR — trading H2's corruption for a leak that was
sitting there unexercised. **Fixing "the hoist" is the wrong target.** The blocker is an
independent interpreter defect in the borrow-or-fresh reassignment shape.

**It is also a BACKEND DIVERGENCE.** `src/generation/dispatch.rs:500-510` (@PLN85) makes
NATIVE free the displaced store on an adopting reassign — its comment names this exact
shape, *"a displaced prior store on reassignment: one store leaked per adopting bind
(`d = choose(..)`)"*. Native is clean on all three leaks found today; the interpreter has
no counterpart. The interp path is `state/codegen.rs::gen_set_first_ref_call_copy`
(:2580), which `OpInitRef` + `OpDatabase`-allocates into the var's slot. Exact mechanism
NOT yet pinned — do not treat this paragraph as a root cause, only as the site to start
from.

**Order of work:** fix that interp gap first; `r10` then goes green and the hoist lands
behind it. The two `AppendSource` expectations and the fuzz-gate control remain
legitimate re-bases AFTER that, never before.

## The lifetimes tool should have caught this — four named gaps

`--show-ownership` reported none of the three leaks found today, and it prints a header
that this session falsified:

```
# store ownership is backend-shared (interp + native lower the same verdict)
```

The two backends emitted different guards for the same binding, so they did NOT lower the
same verdict. Ranked enhancements:

1. **The backend-shared claim is wrong** — a correctness bug in the tool's own output,
   not a missing feature. Either verify the claim or stop printing it. The `j` matrix
   makes the correction sharper than the original wording: it is not "native is right and
   interp is wrong" but *the two backends have different holes* — `j8` leaks on native and
   not on interp, `j1` did the reverse, and `j6` leaks on both.
2. **No notion of runtime-conditional ownership** — a var bound from `t[i] ?? m_none()`
   OWNS its store on one arm and BORROWS on the other. Every leak found here lives exactly
   there, and it is the one thing the tool cannot express: a single static verdict where
   the truth is per-execution. The runtime HAS the concept (`Own::Join`, `OpBindOrCopy`);
   the inspector does not surface it.
3. **Verdict granularity** — `r1`-`r6` all read `Owned` yet behave differently, so the
   classification does not separate the axis that matters.
4. **An unwitnessable `Join` is invisible** — when the base cannot be resolved to a local
   (`j6`: it arrives as `b.items`), `ownership_of` answers `base == u16::MAX` and BOTH
   backends silently fall back to a static split that leaks. That is a knowable-at-compile-
   time hazard the tool could name outright: *"this binding needs a runtime witness and no
   local provides one."* It is the highest-value of the four, because it is the only one
   that would have flagged a defect nothing else here catches.

The `per_iteration_frees` check added this session is what falsified theory 3, so the tool
is not dead weight — it is under-powered specifically on conditional ownership.

## The lifetimes tool — the original wish-list (kept; folded into the four gaps above)

`--show-ownership` reported nothing useful for this program, and `LOFT_STORES=warn`
reported only the downstream leak. The shape is exactly what the inspector exists
for — **a store with two owners, freed by the inner one while the outer still uses
it** — so it belongs in the tool:

- flag a `__lift_N` (or any temp) whose store is also reachable from an enclosing
  owner, i.e. an alias of a caller-provided `__retbuf`;
- report frees inside a loop body of a store allocated OUTSIDE it — a per-iteration
  free of a once-allocated store is almost always this bug;
- surface it in `--show-ownership` output rather than only as a downstream leak
  warning, which names the symptom (`Def×4 not freed`) and not the cause.

That is the attribution upgrade the engineering-rigor skill calls for: the diagnostic
reported an effect with no cause, and three theories were needed to get from the
symptom to the buffer. With the above, the tool would have said it directly.

## The invariant (superseded — see § Refined)

> **A value appended to a container must be owned by that container when the
> appending scope exits.** Either the append copies, or it takes ownership AND the
> scope-exit free for that store is suppressed. Adopting without suppressing is the
> defect.

The adopt is a legitimate optimisation — `p8` shows it working when there is no temp,
and the codebase already reasons about it (`returns_borrowed_view()`,
`body_adopts_call` in `parser/operators.rs`). The bug is the missing half of the
transfer, and only in a loop, where the per-iteration free fires.

## Candidate sites (superseded — the site was neither)

1. The element-append path that chooses adopt-vs-copy for a **call-result** element —
   `p6`/`p7` reach the copy, `p1` does not.
2. Loop scope-exit free emission (`scopes.rs`): the adopted store's free should be
   suppressed once ownership moves into the container.

## Fix options as first ranked (superseded — none of the three was the answer)

1. **Suppress the scope free when the append adopts.** Correct and allocation-free,
   but it must be exact — suppressing one free too many turns silent corruption into
   a silent leak, which is why this needs the ownership analysis rather than a patch.
2. **Copy on a call-result append** (make `p1` emit what `p6` emits). Obviously
   correct and one decision site, at the cost of a record copy per append — the copy
   the via-local form already pays today.
3. Reject the shape — not acceptable; it is ordinary code.

## Validation

`probes/` on **both** backends, hand-computed. `p1`/`p5` must flip to `1 2 3`; every
other cell must stay green (they are the "already correct" side and one of them,
`p8`, depends on the adopt still happening). Plus: no new store-leak warning under
`LOFT_STORES=warn --interpret`, and `loft#496`'s own reproducer re-verified, since
this is its wider form.
