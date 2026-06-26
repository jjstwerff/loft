<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# The borrowed-source over-free class — study + simplification

Companion to [materialisation-site-inventory.md](materialisation-site-inventory.md)
(Layer 3) and [cluster-462](cluster-462-slot-reuse-uaf.md). Written after fixing two
instances and pinning #462 to a third, all the SAME class. Answers the two questions:
*what are the similar paths, and can the class be collapsed?*

## The class (one sentence)

**A value that BORROWS another's store (a view of a vector element, a struct field,
a local container) escapes a scope — via return, bind, or copy — and some site frees
the borrowed store while the escaping value still points into it.** The crash is a
use-after-free; the leak is its mirror (free skipped where it was owed). Every
instance is a site that decided *own vs borrow* (free / copy / adopt / materialise)
and decided wrong, because it re-derived the fact locally instead of reading it.

## The instances (verified this session)

| # | Site | Shape | Wrong decision | Status |
|---|---|---|---|---|
| 1 | `generation/dispatch.rs` first-bind copy gate | `cand = pick(t,i)` where `pick` returns a vector-element view | gated copy on a **type-shape proxy** ("callee has a visible `Reference`/`Enum` param") → missed a `Vector`-param borrow → adopted the view → freed the caller's vector | **FIXED** — read `return_adopts_fresh_store()` (the canonical fact interp already reads) |
| 2 | `parser/control.rs` `copy_borrow_tail_into_retbuf` | implicit-tail `{ b.rows }` (borrowed whole-vector) | captured the borrowed tail into a `__fwd` local with **empty deps** → scope-free freed the borrowed source | **FIXED** — append the borrowed tail inline (no `__fwd`), matching the proven explicit-`return` path |
| 3 | `crawler monsters.loft` `mon_one`/`mon_choose` (**#462**) | `chosen = m; … return chosen` where `m = pool[wj]`, `pool` a **local**; caller does `cand = mon_one(); best = cand` | the returned retbuf **aliases the local `pool`**; `pool` is freed at `mon_one`'s exit → the return dangles → `mon_choose`'s bind-copy reads a freed `tp=76` store | **LOCALIZED, routed** — the #306 "return views a local" path (`return_views_local` → `materialize_view_return`) does not fire for a conditional `chosen = m` view-assign into the retbuf. Reproduces only at slot-reuse scale (cluster-462); pinned by the new `LOFT_UAF_SRC` diagnostic (below). |

All three are the SAME fact decided at three sites: instance 1 at the call-bind, 2 at
the return-delivery, 3 at the in-body view-assign + return. Two were fixed by *reading
the fact instead of re-deriving it*; the third needs the fact to be CARRIED correctly
through a conditional reassignment (it currently isn't).

## The similar paths (the re-derivation surface)

From the Layer-3 inventory + a `grep` of the own-vs-borrow deciders, the fact is
consulted at **~16 sites**, each keying on some subset of
`{ Type::Reference, Type::Enum(_,true,_), Type::Vector }` × `dep.is_empty()` ×
per-site heuristic:

- **Return classification** — `block_result` / `ref_return` arms; `return_views_local`
  + `materialize_view_return` (#306); `materialize_return_into`;
  `copy_borrow_tail_into_retbuf` / `emit_forward_copy_409` (the borrow-vs-own twin —
  one frees its `__fwd`, the other must not); `materialize_vector_arms_into`.
- **Bind / reassign** — interp `gen_set_first_at_tos` / `gen_set_first_ref_call_copy`;
  native `output_set` first-bind copy + `owned_ref_reassign`; the var-to-var same-struct
  deep-copy.
- **Free** — `scopes.rs` scope-free + `make_independent`; the `0x8000` source-free bit on
  `OpCopyRecord` (`do_copy_record`).
- **The two canonical facts already exist** — `Definition::return_adopts_fresh_store()`
  and `returns_borrowed_view()` (`data.rs`) — but are READ at only some sites; the rest
  re-derive (instance 1's deleted proxy; instance 2's empty-dep `__fwd`).

## The return-delivery thicket — concrete inventory (what collapses)

The family that "the one I fixed here" (`copy_borrow_tail_into_retbuf`) belongs to, read
end-to-end. It is **6 emitters + 5 tail predicates + a 5-variant `Delivery` enum**, each a
narrow special-case of "put a value of τ into `__retbuf`, owning it":

**Emitters** (`parser/control.rs`)
| fn | source shape | emission | frees source? |
|---|---|---|---|
| `emit_forward_copy_409` (1067) | owned foreign `#native` store | `__fwd = src; clear(w); append(w,__fwd); w` | yes — `__fwd` (empty deps) is scope-freed |
| `copy_borrow_tail_into_retbuf` (4426) **[fixed]** | borrowed whole-arg / struct-field | `clear(w); append(w, src-inline); w` | **no** — inline, nothing to scope-free |
| `materialize_vector_arms_into` Var-arm (4304) | owned local vector (an arm) | `clear(w); append(w,local); free deps; w` | yes — explicit `OpFreeRef` of deps |
| `materialize_vector_arms_into` Call-arm (4324) | arm's hidden `__ref_N` | substitute buffer onto `w` (NRVO rename) | — (move, not copy) |
| `materialize_return_into` (3972) | Reference/struct view | `OpDatabase(w); OpCopyRecord(src→w); w` | via `0x8000` bit |
| `deliver_mid_vector_returns` (4529) | mid-body vector returns | per-return delivery into buf | — |

**The smoking gun:** `emit_forward_copy_409` and the fixed `copy_borrow_tail_into_retbuf`
are the SAME emitter (`clear; append; w`) differing by **exactly one thing** — the owned
one routes through a `__fwd` local *so scope-exit frees it*; the borrowed one appends
inline *so nothing frees the borrowed store*. That difference **is** the owned-vs-borrow
bit. The Var-arm of `materialize_vector_arms_into` is the same shape with an explicit
free. Three emitters, one parameterised body: `deliver_vector(w, src, owned)`.

**Tail predicates** — all answer "what does the tail borrow / does it own?":
`return_views_local` (3929, borrows a non-arg local), `tail_whole_arg_vector` (4398,
borrows a whole vector arg), `tail_is_struct_field_read` (4365, borrows an arg's field).
These are three reads of the **tail's borrow-set (`deps`)**. (`tail_is_nullable_unwrap`,
4004, is orthogonal — a `__nullable<S>`→`S` *representation* conversion, not an ownership
question; it does NOT fold in.) `classify_vector_delivery` (916) is the switch over them.

**Why they are not already merged — the blocker is the buggy substrate, not taste.**
`tail_is_struct_field_read`'s own comment (control.rs:4350-4364) records that the vector
INDEX-read tail (`fn f(v)->vector { v[0] }`, #426B) is **deliberately excluded** from the
copy funnel: forcing it through `__retbuf` "collides the forward temp's inner-element view
store-nr with a freed sibling store once the caller frees a vector store." That is the
#462 store-reuse hole. So each predicate is *narrowed around the unsafe substrate* — a
naive merge re-exposes #426B / #462. The thicket is accreted scar tissue around a
substrate bug, which is why it keeps growing a special-case per shape.

## Can we simplify it? — YES, and the direction is proven

The two fixes are the template: **each replaced a per-site re-derivation with a read of
the carried fact**, and each made the code SHORTER (instance 1: a 3-line `.any(...)`
param-shape scan → one fact read; instance 2: deleted the `__fwd` local + its set/free).
That is the `OWNERSHIP_MODEL.md` north star in miniature — *dep = the store a local
owns; compute it once, every site reads it.*

The single lever that collapses the class is the **borrow-set (`deps`) as a sound,
propagated fact**, with ONE rule at assignment:

> `v = <expr>` ⟹ `v.deps = ` the borrow set of `<expr>` (the stores `<expr>` views;
> empty iff `<expr>` is freshly owned). Applied uniformly — including a **conditional**
> reassignment (`if … { chosen = m }`) and a join over arms.

With that one fact propagated correctly, every site above stops re-deriving and just
reads:
- `return_views_local` sees `chosen.deps ⊇ {pool}` → materialises (fixes #462 / instance 3).
- the bind copy/adopt reads it (instance 1, already done via `return_adopts_fresh_store`).
- the scope-free skips a borrowed value (no `__fwd` free needed — instance 2).
- `do_copy_record`'s source-free bit is the same fact.

**Why #462 escapes today:** the borrow-set is NOT propagated through the conditional
view-assign `chosen = m` — `chosen` (the retbuf) keeps empty deps though `m` borrows the
local `pool`. So `return_views_local` reads "owned", skips materialisation, and the
retbuf aliases `pool`. The fix is not another per-site patch; it is to make the
assignment rule above hold for the reassign-from-borrow case, then the existing #306
machinery fires unchanged.

## Recommended next step

Land the **assignment borrow-set propagation** (the one rule) as the chokepoint, with
the `mon_one` shape as the driving case. It cannot be blind-patched: #462 reproduces
only at slot-reuse scale, so the guard must be the `LOFT_UAF_SRC` diagnostic on the
crawler corpus (a freed-source read at `do_copy_record` = a fail) plus a synthetic
~200-store slot-reuse stress, not a minimal probe. The two fixes already landed are the
down-payment and the pattern to follow.

## Tooling delivered

`LOFT_UAF_SRC` (`src/keys.rs`, `state/{mod,debug,io}.rs`) — the cheap half of cluster-462
tool-gap #1: records each store's freeing pc and, when `do_copy_record` reads a still-freed
SOURCE, reports the source store + the line that freed it + the copy site + both function
`d_nr`s. Unlike full `LOFT_UAF` it skips the per-op frame scan, so it runs to the fault at
real-consumer scale. It is what pinned instance 3 to `mon_one:258` → `mon_choose:269`.
