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
