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

---

## See also

- Method: [CODEGEN_METHOD.md](../../CODEGEN_METHOD.md)
- The target bytecode + rungs: [stage-c-move-convention-design.md](stage-c-move-convention-design.md)
- Mechanism + the 9 attempts: [cluster-II-slot-init-dominance.md](cluster-II-slot-init-dominance.md)
- The Deps model this extends: [DEPS_INVENTORY.md](../../DEPS_INVENTORY.md) · [LIFETIME.md](../../LIFETIME.md)
