<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Stage C — type-system design: heap-return ownership as a type fact

Per [CODEGEN_METHOD.md](../../CODEGEN_METHOD.md): the `has_ref_params` heuristic at
the call site and the `returned_var` structural walk in scope analysis are the
*symptoms*. The *diagnosis* is that **heap-return ownership is not a computed type
fact** — so both pieces of codegen are forced to re-derive it, per site, and both
get it wrong on shapes like `match` returns. This doc designs the missing fact, so
codegen collapses to mechanical reads.

Builds on the typed `Deps` work ([DEPS_INVENTORY.md](../../DEPS_INVENTORY.md)) and
the dep/lifetime model ([LIFETIME.md](../../LIFETIME.md)).

## 1. What the two codegen consumers need (and re-derive today)

| Consumer | Question it must answer | What it does TODAY (the symptom) |
|---|---|---|
| **Caller** (`gen_set`, `a = f(args)`) | adopt the result, or copy it into `a`'s own store? | re-derives via `has_ref_params` + result type + borrowed-view checks — a heuristic forest |
| **Callee** (`get_free_vars`, scope exit) | free local `L`, or is it transferred out as the return? | `owns(L) && !in_ret(L)`, where `in_ret` comes from `returned_var` — a structural walk that returns `u16::MAX` (→ "not returned") for a `match`/`if` return, so the returned arm buffers get freed (the #405 / cbor / probe-05 bug) |

Both questions are the SAME fact viewed from two ends: **where does a heap value's
storage come from, and does ownership transfer on return?** Make it one computed
fact; both reads become trivial.

## 2. The fact: return-ownership origin, carried on the type's `Deps`

A heap value's `Type` already carries `Deps` (typed: `DepEntry::Attr(a)` |
`DepEntry::CalleeFrame(w)`). Define its meaning for a function's **return type**
(`Definition.returned`) precisely:

- **`Deps::none()` → OWNED / TRANSFERRED.** The callee returns a freshly-owned
  store; ownership moves to the caller. Caller **adopts**; callee **does not free**
  it. (e.g. `mk(n) { return [n] }`, and `enc` once fixed.)
- **`Deps` = `{Attr(a), …}` → BORROWS attribute(s) a.** The returned ref aliases
  param `a`'s storage (or the hidden `__retbuf` out-param, which is itself an attr).
  Caller must **copy** to own; callee **does not free** (it never owned it).
  (e.g. `fn id(v: vector) -> vector { return v }` borrows attr `v`.)

That is the whole fact. It is the existing `Deps` mechanism with a *defined,
complete* semantics for returns — not a new type kind.

## 3. The computation (this is the actual type-system change)

Compute `Definition.returned`'s `Deps` **correctly and completely for every return
shape**, once, during two-pass type resolution. The same traversal yields the
callee's **return-source set** (the locals whose stores flow out as the return).

### 3a. Per-expression ownership origin

`origin(e) : Deps` for a return expression `e`:
- fresh literal / allocation (`[..]`, `S{..}`, a call whose own return is owned) →
  `Deps::none()` (owned).
- a parameter, or a field/element/view rooted in a parameter → `{Attr(that param)}`.
- a call returning a borrow of one of THIS function's params → propagate that attr.

### 3b. The match/if reconciliation rule (the missing piece)

For `e = match/if` with arms `e1..en` (the shape `returned_var` fails on):
- `origin(e) = reconcile(origin(e1) .. origin(en))`:
  - **all arms owned → owned** (`Deps::none()`). ← the `enc` case: every arm is a
    fresh `[literal]`, so the return is owned/transferred.
  - **any arm borrows attr `a` → the union of borrowed attrs** (conservative: a
    value that *might* be a view is treated as a view, so the caller copies).
- **return-source set** gets EVERY arm's terminal buffer (not just one unified
  var). For `enc`: both `__vdb_1` and `__vdb_2` are return-sources. (Whether the
  arms are physically unified into one buffer is a codegen choice; the OWNERSHIP
  fact is that all arm buffers are transferred-out and must not be freed.)

This is what `returned_var`'s single-`u16` structural walk cannot express — it
needs a SET and a reconcile, computed over the real return shape.

### 3c. Where it lives

In the parser/type-resolution that already fills `Definition.returned` and the
per-local dep types — extended so the return `Deps` and the return-source set are
the authoritative, complete facts. The `unify_if_branches_work_refs` machinery
becomes an *implementation detail* of materializing the arms into a buffer; it no
longer carries the ownership decision (the type does).

## 4. Consumption — both codegen sites become mechanical

- **Callee free-decision** (`get_free_vars`): free `L` iff `owns(L) && L ∉
  return_sources`. No `returned_var` walk; `return_sources` is the computed set,
  correct for `match`. → `enc`'s arm buffers are in the set → not freed → the
  caller adopts a live store. The match-return-freeing bug is gone by construction.
- **Caller adopt-vs-copy** (`gen_set`): read `f.returned.deps`:
  - `Deps::none()` (owned) → **adopt** (`PutRef`); ownership transfers.
  - borrows an attr → **copy** (`AppendVector`/`OpCopyRecord` into `a`'s own store).
  No `has_ref_params`, no borrowed-view re-derivation — one read.

The complexity that lived in codegen is deleted; it became one type fact + one
reconcile, computed once.

## 5. Why this is the structural fix for the whole class

- **#405 / probe 05 (`a=enc(); b=enc()`):** `enc`'s return is owned → not freed →
  `a` adopts a live store → the next call allocates a distinct store (the live one
  isn't recycled) → no aliasing. Falls out of the fact; no special case.
- **cbor `encode_map`:** `encode`'s return is owned (its arms write `__retbuf` and
  are transferred), so held `ki`/`buf`/value results are each owned/distinct — the
  multi-live interaction dissolves once nothing is freed-on-return. (The separate
  `entries[i].key` enum-field READ corruption is a different fact — record-field
  ownership/layout — and gets its OWN type-fact design; do not assume this closes
  it. See cluster-II § scope correction.)
- **interp == native:** both backends translate the SAME return `Deps` →
  mechanically the same decision. The attempt-8 native E0425 was a hand-unification
  leaving dangling vars; driven by the computed return-source set instead, there is
  no dangling var to mistranslate.

## 6. Validation (the method, in order)

The rung-0 bytecode must FALL OUT of the fact, not be coaxed:
1. Compute + dump `enc.returned.deps` (expect: owned / `Deps::none()`) and the
   return-source set (expect: `{__vdb_1, __vdb_2}`).
2. Callee: `get_free_vars` emits NO `OpFreeRef` for the return-sources (diff vs the
   broken dump — the two `OpFreeRef(__vdb_*)` disappear).
3. Caller: `f.returned.deps == none` → adopt; `a=enc(k0); b=enc(k1)` → distinct
   stores → `1 3` on BOTH backends.
4. Then grow the rungs (probe 05 → multi-arm → nested-loop → full functions), each
   gated both backends + leak + suite.

## 6b. The MINIMAL change (rung 0) — just the callee return-source fact

The full §2–4 design (return `Deps` = owned vs borrows-attr, read by BOTH sides) is
the end state. But rung 0 (`a = enc(k0); b = enc(k1)`) needs only a fraction of it,
because **enc's return is OWNED** — so the caller's *existing* adopt is already
correct; the only thing wrong is that the **callee frees what it transfers out**.
So the minimal delta is callee-side only:

> **Compute the function's "return-source set" — the locals whose heap store is
> transferred out as the return — correctly for EVERY return shape, and mark each
> "do not free at scope exit." The only gap vs today is `match`/`if`.**

Concretely, the smallest realization:

- **Representation — reuse `skip_free`, add NO new type field.** A return-source
  local is exactly one that must not be freed at scope exit, which is what
  `skip_free` already means and what `get_free_vars`'s existing
  `(owns || work_ref) && !skip_free` gate already consumes. So the change is: mark
  each return-source `skip_free`.
- **Computation — extend the return-tail analysis from a single var to a SET.**
  Today `returned_var(e)` yields one `u16` and returns `u16::MAX` for an `If`/`match`
  whose arms differ (→ nothing marked → arm buffers freed). Minimal change: over
  the return expression, collect the terminal buffer of each path — for `Var` /
  `Block` / `Insert` / `Span` the terminal; **for `If`/`match`, the UNION of all
  arms' terminals** (the one missing case). `set_skip_free` each.
- **Consumption — unchanged.** `get_free_vars` already skips `skip_free` locals.
  No edit there. The callee stops freeing `__vdb_1`/`__vdb_2`; the caller's existing
  `PutRef` adopt now binds a LIVE store; the second call allocates a distinct store
  (the live one isn't recycled) → `a`,`b` distinct.

What the minimal change does NOT include (deferred to later rungs):

- **The caller-side `Deps` read (adopt-vs-COPY).** Needed only when a function
  returns a BORROW of a param (`return v`); rung 0's return is owned → adopt. Add
  when a rung exercises a borrowed return.
- **Arm unification into one buffer.** Whether the callee must materialize all arms
  into a single buffer (so the caller adopts a consistent store) is a *codegen*
  question for the bytecode rung, separable from this type fact. Attempt 8's native
  E0425 came from rewriting/unifying vars (dangling refs), NOT from the `skip_free`
  marks — so the minimal change deliberately marks-without-rewriting, and the
  rung-0 bytecode work settles whether unification is additionally required. Verify
  empirically (open question below), don't assume.

This keeps the first change tiny and additive: one set-valued traversal + existing
`set_skip_free`, no new type field, no caller edit, no var rewriting.

### TESTED (the codegen is the test) — interp: ENOUGH; native: needs a function-scoped return buffer

Implemented the minimal change and observed the bytecode:
`collect_return_sources(expr)` (set version of `returned_var`, `If`/`match` →
union of arm terminals) + `set_skip_free` on each source's BACKING BUFFER, with the
owned-vs-borrowing distinction the validator flagged:

- **directly-owned terminal** (`dep.is_empty()`) → mark the terminal itself (it IS
  the freed store; a multi-arm owned return would otherwise free it → UAF).
- **borrowing terminal** (`_vec_N["__vdb_N"]`) → mark the DEP (`__vdb_N`, the
  work-ref `get_free_vars` actually frees), NOT the terminal — marking the terminal
  makes the native generator skip *declaring* it (`E0425 var__vec_N not in scope`).

Result:

- **INTERP — ENOUGH.** `enc`'s two `OpFreeRef(__vdb_*)` disappear (verified in the
  IR; the return wraps as `return {…}`). Rung 0 (the *direct* `a=enc(k0);
  b=enc(k1); c=enc(k2)`) → `1 3 5`. Probe 05 passes. audience_crystal 02/03 no
  regression. The minimal type-fact change, alone, fixes the interpreter.
- **NATIVE — NOT enough.** Same IR fails to compile: `E0425 cannot find value
  var__vec_1` — the return now yields **arm-scoped** `_vec_N` vars, and Rust needs
  the returned value to come from a var that is **in scope at the return** (a
  function-scoped buffer). So the previously-"separable" arm-unification question
  is **answered: native REQUIRES it; interp does not** (the interpreter is
  slot-based, no lexical scope). The reverted attempt-8 unification broke native by
  *rewriting* vars; the correct native step is to materialize every arm's result
  into ONE function-scoped buffer (the `__retbuf`, exactly as cbor's head-call arms
  already do via `one_buffer_chain`) so the return is `return __retbuf` — no
  arm-scoped var, no rewrite.

**So rung 0 splits cleanly into two sub-steps, both now specified:**
1. **interp (callee return-source skip_free)** — the minimal change above; proven.
2. **native (arm → `__retbuf` materialization)** — make simple-literal arms write
   the function's `__retbuf` like the head-call arms do, so the return references
   one in-scope buffer. Then both backends emit the proven form.

(The change is reverted on the branch — it breaks native *compile*, so it can't
land until sub-step 2 lands with it.)

## 6c. Rung: borrow-of-param return — the caller-side answer (TESTED)

The decisive test for "is the type good enough, or do we need more detail":
`fn id(v: vector<u8>) -> vector<u8> { return v; }`, then `a = id(x)`.

- **Runtime, both backends:** `a` **aliases** `x` — after `x += [9]`, `len(a)`
  becomes 4 (not 3). Binding a borrow-of-param result *adopts* the param's store
  instead of COPYING it — a value-semantics violation (a plain `b = a` deep-copies;
  `a = id(x)` does not). A real latent bug, same ownership family.
- **The type carries NO borrow fact:** `id`'s introspected signature is
  `-> vector<integer(0, 255)>` with an **empty return dep** — it does NOT say
  "borrows attr `v`", even though it returns `v`. A borrow-return is
  indistinguishable from an owned-return at the type level.

So the caller, reading the (empty) return dep, concludes "owned → adopt" and
aliases. **This is the one place we genuinely need more detail.** The `Deps`
MECHANISM exists (it can express `DepEntry::Attr(v)`), but for RETURNS it is **not
populated**: a return that aliases a parameter must record that attr on the return
type. The fix is therefore "compute + carry the return-ownership dep correctly"
(owned ⇒ empty; borrows param `v` ⇒ `{Attr(v)}`), not a new type kind — but it IS a
real addition of detail to what the return type states today (which is nothing).

Net for the evaluation:
- **Callee side (free-on-return, rung 0):** type system already good enough — the
  facts were present; the gap was an incomplete traversal (`returned_var`). No new
  detail.
- **Caller side (adopt-vs-copy):** NOT good enough today — the return type omits
  the borrow-of-param fact, so adopt-vs-copy can't be decided and borrow-returns
  alias. Needs the return-ownership dep **computed and carried** (the missing
  detail). Then the caller reads it: empty ⇒ adopt, `{Attr(a)}` ⇒ copy.

## 6d. First-step attempt — blocked by the unresolved (`"??"`) dep + the branch thicket

Tried the smallest green-both behavior fix: copy an implicit param-return
(`fn f(v) -> vector { v }`) into the return buffer (the proven `fwd_copy_409`
shape), so the result is owned and the caller stops aliasing the arg. It **did not
fire**, and the reason is the foundational obstacle this whole effort names:

- The implicit-tail return carries a **`"??"` dep, which is `Type::Unknown`**
  (`data.rs:4644`) — the ownership dep is not merely wrong, it is **UNCOMPUTED**.
- A non-empty (`"??"`) dep routes the return through the `else` (`ref_return`)
  branch, NOT the `ls.is_empty()` branch the edit targeted — so the fix never ran.
- The vector return-handling is a **multi-branch thicket**
  (`BlockTail` vs `MidReturn`; within `BlockTail`: `ls.is_empty()` work-ref
  recovery vs native-forwarder vs `else`), and explicit `return v` is yet another
  path (`MidReturn`). A targeted edit lands in one branch while the case flows
  through another.

This is exactly the OWNERSHIP_MODEL diagnosis made concrete: the ownership fact is
**not computed** (renders `Type::Unknown`/`"??"`), and the decision is **scattered
across duplicated paths**. Conclusion: the genuine first small step is NOT a leaf
behavior fix — it is **foundational**: *resolve the `"??"`/`Unknown` return dep
into a real, computed ownership dep, and/or consolidate the return-handling paths
to one*, so that a single place owns the answer and a leaf fix can land in it.
That consolidation is itself the first down payment on the beacon (one path, one
fact). Reverted; no clean green-both behavior fix is reachable until the dep is
resolved.

## 6e. Trace of the return dep — corrected map (the `"??"` was a render artifact)

Instrumented the raw `Definition.returned` deps (not the misleading display):

- **implicit `fn idt(v) -> vector { v }`:** `returned = Vector(.., Deps{items:[0]})`
  — **the borrow fact IS computed** (attr 0 = `v`). The `"??"` in the
  `dump_fn_signature` output is a *render artifact* (that dump's var table lacks
  names at print time; `introspect.rs` renders the SAME dep as `["v"]`, and the
  block-result type shows `["v"]`). My earlier "uncomputed/`Type::Unknown`"
  reading (§ 6d) was WRONG — corrected here.
- **explicit `fn idr(v) -> vector { return v }`:** `returned = Vector(.., Deps{items:[]})`
  — **empty; the borrow fact is NOT computed** on the explicit-return path.

And at runtime BOTH still alias (`a=idt(x); x+=[..]` mutates `a`), even though
`idt`'s dep is correct. So:

**The precise gap map (replaces § 6c/6d guesses):**
1. **Computation — explicit returns.** `return v` leaves the dep empty; the
   implicit tail computes it. Close the explicit-return path (or funnel both to one
   computation) so the borrow dep is always present. *(small, parser-side)*
2. **Consumption — the vector caller.** Even with the correct dep `[0]`, `a = idt(x)`
   ALIASES — the vector caller does not read the return dep to decide copy. The
   **Reference** caller already does (`ids(s)` copies, green both backends), so
   there is a working template; the vector caller needs the analogous copy. This is
   the recurring vector-deep-copy, and its blocker is the native side (a vector copy
   = fresh store + `OpAppendVector`, where the `rec_tp`/scoping must be right on
   both backends).
3. **Owned returns freed (cluster II).** For an OWNED return (`enc`'s arms), the
   callee FREES it, so even a correct adopt aliases — the callee-skip-free fix,
   whose blocker is the native `E0425` (arm-scoped return var).

So the fact is largely *computed* already; the open work is **(1) finish the
explicit-return computation, (2) make the vector caller consume the dep
(copy-on-borrow), and (3) stop the callee freeing owned returns** — items 2 and 3
both gated on native-generator work (function-scoped return buffer / vector copy).
That native-generator capability is the shared prerequisite, and is the honest next
focus before the leaf fixes can land green-both.

## 6f. Item-2 execution — interp DONE via `OpCopyRefOrNull`; native is a SEPARATE generator

Executed the borrow-return consumer (item 2). Key results, all verified:

- **`OpCopyRefOrNull` deep-copies a vector** (`copy_claims` has a `Parts::Vector`
  arm → `copy_claims_seq_vector`). So the vector caller-copy uses the SAME op as
  the Reference path — attempt-7's `OpAppendVector`/`rec_tp` was the wrong tool.
- **The Reference deep-copy is green-both** (a struct param-return `ids(s)` copies
  on `--native`: `a_xs_len=3`). Confirmed template.
- **Implemented:** refactored `gen_set_first_ref_call_copy(.., tp_nr: u16)` and
  added a `gen_set_first_at_tos` arm routing a Vector result whose callee's
  `returned` dep is non-empty (a borrow-return) through it with the vector's
  `name_type`. Result: **`idt` copies on INTERP** (`a_len=3`, was 4) — the borrow-
  return aliasing is fixed on the interpreter.
- **Two blockers remain → not green-both, reverted:**
  1. **Native is a SEPARATE generator.** `state/codegen.rs::gen_set_first_at_tos`
     drives the INTERPRETER bytecode; `--native` is generated by `src/generation/`,
     which has its OWN `gen_set` and did NOT get the arm — so native `idt` still
     aliases. **This is the structural reason interp fixes don't reach native:
     every codegen fact needs BOTH generators.** The native equivalent of the arm
     (route a vector borrow-return through the runtime `OpCopyRefOrNull`/
     `OpCopyRecord`, which already deep-copies vectors) is the missing piece.
  2. **Leak (dep-strip).** After the copy, `a` owns a fresh store but its TYPE
     still borrows the arg, so `get_free_vars` doesn't free it → 1 leaked store.
     Needs the scan_set dep-strip (mark `a` owned), mirroring the Reference path's
     `make_independent`.

So item 2 is **half-landed**: interp mechanism proven (OpCopyRefOrNull copies
vectors), and the two remaining pieces are precise and small — (a) the
`src/generation/` arm, (b) the scan_set dep-strip. Per always-green it can't land
until both are in (native correctness + no leak). This also re-frames the whole
plan: each fact is a **paired** change (interp `state/codegen.rs` + native
`src/generation/` + shared `scopes.rs`), and "green-both" means all three.

## 6g. CRITICAL path finding — `block_result`'s vector arm is NOT enc's return path

While executing the owned-return fix (probe 05), an instrument proved a structural
blocker: the `parser/control.rs` `block_result` **vector return arm** (the
`} else if let Type::Vector(elm, ls) = t {` at ~line 695, where `ref_return` /
`native_forwarder` / the #410 materialise live) **fires for ZERO functions** in the
`a=enc(k0); b=enc(k1)` program (`LOFT_PLN85_DBG` printed nothing for `n_enc` — or
any fn). So `enc`'s `match`-return is processed by a DIFFERENT, not-yet-located
return-handling path.

This means several recent fix attempts (the param-return materialise, the
match-tail `materialize_vector_return_into`) were edited into a path `enc` never
takes — which is why they never fired. **Before ANY further owned-return fix, the
real path enc's vector match-return flows through must be found** (candidates: a
separate explicit/implicit return handler, the `RetSite::MidReturn` path, or
codegen-side handling that bypasses `block_result`). Trace it with a breadcrumb on
the function that actually rewrites `enc`'s tail / sets its `OpFreeRef(__vdb_*)`.

Net: the investigation is sound and the fact-map (§ 6e) holds, but the
*implementation site* for the owned-return path was wrong. The next session's first
move is path-location, not another edit.

## 6h. BREAKTHROUGH — the working mechanism (probe 05 green BOTH backends), + its two remaining edges

This session found the mechanism that makes the cluster-II owned-return correct on
**both backends** for the first time in the whole investigation. Two coordinated
changes did it:

1. **scopes.rs — `collect_return_sources` + skip_free.** `returned_var` collapses a
   `match`/`if` to `u16::MAX`, so the arm buffers get freed (the bug). A SET
   collector (`collect_return_sources`, union of all arms incl. `If`/`Insert`/`Block`
   terminals) marks every return-source do-not-free: owned terminal → mark it;
   borrowing terminal (`_vec_N["__vdb_N"]`) → mark its dep `__vdb_N`. Result on
   INTERP: `enc`'s `OpFreeRef(__vdb_*)` disappear → probe 05 = `1 3 5`.
2. **emit.rs — brace a `Block`/`Insert` return value (a real native-generator bug).**
   With the frees gone the return becomes `return {block}`, and the native generator
   emitted `return let …; …` **unbraced** — invalid Rust, arm vars out of scope
   (E0425). Wrapping as `return { … }` fixed native → probe 05 = `1 3 5` on `--native`
   too, fully clean (no leak on native).

**Verified:** probe 05 `a=1 b=3 c=5` on BOTH backends; single-call + `mk()`×3 clean.
This is the design (move-on-return) proven end to end.

**Why it is reverted — the two edges still to close:**

- **Edge A — over-broad (the blocker).** `collect_return_sources` fires for EVERY
  function's return, so it over-marks **keyed-collection** (`hash`/`sorted`/`index`)
  and enum returns whose buffers SHOULD be freed → ~13 suite regressions, crashing
  at `allocation.rs:562` / `keys.rs:295` (the #405 UAF/OOB shape) + audience_crystal
  "index 65535". **Narrowing:** gate the marking to the case `returned_var` actually
  misses — `returned_var(expr) == u16::MAX` AND the return type is `Vector` — so
  single-var and keyed/enum returns keep their existing (correct) handling untouched.
- **Edge B — interp leak (3 element buffers).** `enc`'s arms deliver via local
  `_vec_N`/`__vdb_N`, NOT the caller's `__retbuf`, because
  `unify_if_branches_work_refs` only unifies `__ref_`/`__rref_` terminals (not
  `_vec_`). skip_free then suppresses the callee free but the store isn't NRVO-moved
  into `__retbuf`, so on interp the element buffers (`kt=65535`) orphan (native's
  adopt frees them, so native is clean). **Fix:** extend the if-branch unification to
  deliver vector match-arms via `__retbuf` (true NRVO), so there is no local buffer
  to skip_free at all — which also subsumes Edge A's risk.

**Next move (precise):** re-apply the two changes, narrow Edge A (gate on
`returned_var==MAX && Vector`), re-run the 13-test regression set + probe 05 both
backends; then close Edge B by NRVO-delivering the vector match-arms into `__retbuf`.
The native brace fix (emit.rs) is independently correct and can land on its own
once a producer exercises it.

### 6h-update — Edge A CLOSED (narrowing verified), Edge B is the last item

Re-applied with the narrowing (marking gated on `ret_var == u16::MAX` AND per-source
`Type::Vector`). Verified:

- **probe 05 = `1 3 5` on BOTH backends** (unchanged by the narrowing).
- **Full suite CLEAN** — all 13 prior regressions gone (p300 hash/sorted/index,
  p188, p295, leak_cases, native_scripts, plan25_e2_hash, par_struct, audience_crystal
  library_suite). The only red was `engine_host_kernel` (2 threading tests) which
  **passes 14/0 in isolation** → flaky, not this change.

So Edge A is closed: the keyed/enum/single-var returns keep their correct handling;
only vector multi-arm match-returns are touched. **Edge B (the interp-only 3-store
leak) is now the single remaining item** before probe 05 graduates: `enc` delivers
via local `_vec_/__vdb` (skip_free'd) rather than NRVO-moving into `__retbuf`, so
interp orphans the vector backing stores (`kt=65535`) while native's adopt frees
them. The fix is to NRVO-deliver the vector match-arms into `__retbuf` (extend the
if-branch unification to vector `_vec_` terminals), eliminating the local buffer
entirely so there is nothing to skip_free or leak. The change is committed as WIP on
the branch (suite-clean, both-backends-correct); it is NOT a finished landing until
Edge B closes (no-leak gate).

**Edge B pinned precisely (instrumented):** the interp leak is exactly **3 vector
*backing* buffers** (one per call) — `store#9/#11/#13`, `known_type=65535`,
100-word element-capacity stores (the growable backing `pre_alloc_vector` mints,
separate from the `kt=68` header). A SINGLE `enc` call is leak-free (0); the leak is
per-call and only when the result is held. The vector *header* (a/b/c) IS freed by
`main`, but its backing is not — because `enc` delivers via a transferred LOCAL
(`__vdb`, skip_free'd) rather than NRVO-moving into `__retbuf`: `free_named` on the
header does not cascade to the backing for a transferred-local vector, whereas an
NRVO'd vector (`mk()`, header == caller's `__retbuf`) frees its backing correctly,
and native's adopt path frees it on both. So Edge B is NOT a free_named bug to patch
(adding a header→backing cascade there would double-free the NRVO'd common case); it
is the same missing fact — **deliver the vector match-arms via `__retbuf` (NRVO)**,
so the result is the caller's buffer with a properly-owned backing and there is no
transferred local to leak. Path note: `enc`'s match-return does NOT flow through
`block_result`'s vector arm (verified: zero user `n_` fns reach it) — the NRVO must
be injected on the match-handling path (or `unify_if_branches_work_refs` extended to
`_vec_` terminals), which is the precise next-session target.

## 7. Open questions to verify against the implementation

- Exact current contents of `Definition.returned` for `enc`/`mk` (is the dep empty,
  `??`, or attr-tagged today?) — pin before changing the computation.
- Whether `return_sources` is best a new field on the function / a derived analysis
  result, vs. reusing `skip_free` marks on those locals (the cheap path: the
  computed set just calls `set_skip_free` on each return-source — then `get_free_vars`'s
  existing `!skip_free` gate consumes it with no new field).
- Reconcile for nested heap (a returned `vector<vector>` / struct-with-ref-fields):
  the origin of inner elements vs the outer container — likely the same rule applied
  structurally; confirm with a rung.
- Native generator: confirm it consumes the same `returned.deps` / return-source
  facts (not a parallel derivation) so the two backends cannot diverge.

## 6h-update-2 — Edge B fix attempts mapped (dead ends ruled out)

A full session narrowed the Edge-B fix to a precise shape and ruled out the
tempting wrong turns — recorded so the next session does not re-walk them:

- **Why the type-keyed vector arm never fires for enc:** its match tail types as
  `t = Never` (diverging arms), and `block_result`'s vector handling matches on `t`,
  not the declared `result` (=Vector). A new branch keyed off `result` when
  `t==Never && tail-terminal-is-branch` DOES fire.
- **Whole-tail materialize** (`append(__retbuf, <whole If>)`): **interp goes fully
  clean** (correct + no leak) but **native E0425** — the `If` sits in
  `let _av_s = <If>` expression position, and `pre_declare_branch_vars` emits `let`
  STATEMENTS that cannot live there. Dead end unless the native append-source
  pre-declares branch vars at a statement scope.
- **Per-arm materialize** (rewrite each arm to `clear+append(buf,_vec)+buf`): the
  descent used `Block.operators.last()`, but an arm's result `_vec_N` is the block's
  tracked result; got the shape wrong and leaked header + backing. The delivery ops
  must be APPENDED to the arm (after its build ops), not replace a terminal.
- **Relaxing `unify_if_branches_work_refs` to accept `_vec_` terminals** (so enc's
  arms unify and ride the return-`if` path): the arms DO unify, values stay correct
  on both backends — **but the interp leak is unchanged** (3 `kt=65535` backings).
  Unification alone is not sufficient.
- **Conclusion — the leak is interp store-lifecycle, not parser/codegen.** Values are
  correct on both backends; `free_named` frees the `kt=68` vector HEADER but not its
  separate `kt=65535` backing (from `OpPreAllocVector`) for a *transferred* vector
  (an NRVO'd vector frees its backing; native's adopt frees it). The durable fix is
  (a) true NRVO delivery — per-arm append into `__retbuf` with the correct arm-result
  accessor (not `operators.last()`) — or (b) an interp-side header→backing cascade in
  the vector free path (idempotent via the `store.free` no-op, scoped to NOT
  double-handle the NRVO'd common case). A focused interp-lifecycle item, separate
  from the now-shipped parser/codegen correctness.

### 6h-update-3 — Edge B is an interp/native EXECUTION divergence (not a parser fix)

Final, decisive finding after re-running the NRVO materialize end-to-end:

- With the whole-tail NRVO materialize applied, **interp goes FULLY CLEAN** (correct
  `1 3 5`, no leak) — proof the move-on-return delivery is right.
- But the **native generator ignores the materialized IR**: enc's native body emits
  `return <decls + if>` (the raw match) with **no `OpClearVector`/`vector_add`** — and
  unbraced (`return let mut var__vec_1 …`, E0425). So the parser-level materialize is
  interp-only-effective; native re-derives the function-body return from the raw
  structure. That makes the materialize a HARD native regression → reverted.
- Crucially, the shipped milestone (skip_free, no materialize) is the opposite split:
  **native is fully clean (no leak)** while **interp leaks** the 3 `kt=65535` vector
  backing stores — even though both backends run the SAME IR and the SAME `free_named`.
  So Edge B is not a missing IR transform; it is a genuine **interp-vs-native
  execution divergence in the vector free path** (native's free of a transferred
  vector reaches its separate backing store; the interpreter's does not).

So the durable close is one of: **(a)** teach the native generator to consume the
materialized IR for vector match-returns (then the NRVO interp-clean result lands on
both), or **(b)** fix the interpreter's vector free to reach the transferred backing
store the way native already does. Both are dedicated, suite-validated changes —
NOT a quick edit on top of the clean milestone, which is why this session ships the
both-backends-correct milestone and leaves Edge B (the interp no-leak gate) as the
single bounded follow-up.

### 6h-update-4 — Edge B ROOT-CAUSED (backtrace): `init_ref`'s eager work-ref store

A `Store::new(100)` backtrace pinned the 3 leaked `kt=65535` stores exactly:

```
Store::new(100) ← Stores::database_named ← State::init_ref ← execute_argv
```

`OpInitRef` initializes each `__ref_N` work-ref (the `__retbuf` passed to a heap
return) via `self.database.null()` → `database(u32::MAX)` → an **eager 100-word
`Store::new`**. So every work-ref gets a real pre-allocated store, NOT a null
sentinel. The NRVO contract is that the callee FILLS that buffer and returns it.

- **native** honours it: `enc` writes the buffer → it becomes `a` → freed by the
  caller. Clean.
- **interp** violates it: `enc`'s `match` arms build their OWN `__vdb` and return it
  via the eval stack, ignoring the passed `__ref_N`. So `__ref_N`'s eager store is
  orphaned and leaks (the caller's `OpFreeRef(__ref_N)` frees the rebound `__vdb`,
  not the original buffer store).

So the leak is a precise interp/native NRVO divergence: **interp's vector
match-return doesn't deliver into the caller's `__retbuf`.** The parser materialize
(§6h-update-3) makes interp deliver correctly (verified leak-clean), and the IR
genuinely carries it (`one_buffer_vec_copy` / `OpClearVector(__retbuf)` /
`OpAppendVector(__retbuf, …)` present) — but the **native generator mishandles that
exact IR**: for `OpAppendVector(__retbuf, <if-source>)` in return position it emits
the if-source (arm-scoped `_vec_`, unbraced) instead of the materialized block →
`error: expected expression, found let`. So the materialize is interp-correct but a
native compile regression.

**Three candidate closes (for a dedicated session), in rough risk order:**
1. **Per-arm Var-source materialize** — deliver into `__retbuf` INSIDE each arm
   (`{…build…; clear(buf); append(buf, _vec_arm); buf}`) so the append source is a
   bare `Var` (native-safe) and the `If` yields `buf` (a return-`if` native handles).
   Needs the arm's result located via its tracked result, not `operators.last()`,
   AND the per-arm `__vdb` intermediates freed on interp (they leaked when tried).
2. **Fix the native generator** to emit the materialized `OpAppendVector(buf,<if>)`
   block correctly (brace + scope the if-source arm vars) — then the §6h-update-3
   whole-tail materialize lands on both.
3. **Lazy work-ref allocation** — `init_ref`/`null()` write a null sentinel instead
   of eagerly allocating, so an unfilled buffer has no store to leak. Smallest at the
   leak site but largest blast radius (every `DbRef` var / NRVO fill path must handle
   a sentinel buffer); validate the whole suite.

The shipped milestone (skip_free, both-backends-correct, suite-clean) stands; this is
the bounded, root-caused follow-up.

---

## See also

- Method: [CODEGEN_METHOD.md](../../CODEGEN_METHOD.md)
- The target bytecode + rungs: [stage-c-move-convention-design.md](stage-c-move-convention-design.md)
- Mechanism + the 9 attempts: [cluster-II-slot-init-dominance.md](cluster-II-slot-init-dominance.md)
- The Deps model this extends: [DEPS_INVENTORY.md](../../DEPS_INVENTORY.md) · [LIFETIME.md](../../LIFETIME.md)
