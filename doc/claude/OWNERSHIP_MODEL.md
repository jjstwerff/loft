<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# OWNERSHIP_MODEL.md — loft's ownership/borrow system: the north star

This is the beacon the language is steering toward. It is *aspirational*: loft does
not fully implement it yet. Its purpose is to give every store-lifetime / codegen
decision one place to point at, and to turn a recurring class of sev:high bugs into
a single, finite migration.

## The beacon (one sentence)

> **`deps` should become a sound, complete, statically-computed ownership/borrow
> system — loft's borrow checker — from which every store-lifetime codegen decision
> (free placement, adopt-vs-copy, move-vs-clone, drop) derives MECHANICALLY.**

Corollary (from [CODEGEN_METHOD.md](CODEGEN_METHOD.md)): a store-lifetime bug is
then never a *codegen* bug — it is a **hole in the ownership computation**. Fix the
fact, not the generator.

## The law — reference by default, `&` to reassign back

A binding or parameter to a heap value — struct, vector, element — **aliases** the
source; it does not copy. In-place mutation *through* the alias reaches the original,
because they share one store: `o.field = x`, `o.v[i] = y`, and `a = vv[0]; a[i] = z` all
write back — no annotation. That is loft's actual model (verified on both backends), and
`a = vv[0]` being a view is a **feature**, not the #426 bug it was first read as.

The one thing an alias cannot do silently is **replace the whole binding**:
`o = Obj{...}` (rebinding the variable to a fresh value) is a **local rebind** — it does
nothing to the source. To make a reassignment write back, prefix the source with **`&`**:

> **`&` means "reassigning this binding writes back to the source variable."** One
> meaning, uniform across scalars and structs.
>
> | operation | plain | `&` |
> |---|---|---|
> | in-place mutate (`o.field = x`, `o.v[i] = y`) | writes through (heap is shared) | same |
> | **reassign the binding** (`o = Obj{...}`) | local rebind — source untouched | **writes back to the source** |
> | scalar (`n = n + 1`) | local (scalars are by-value) | writes back |

- **`&` is load-bearing exactly when the body reassigns the binding** — and nowhere
  else. A `&` on a struct whose body only mutates fields is **redundant** (the mutation
  already writes through). That redundancy is a lint (W4: "`&` here has no effect; `&`
  only matters when you reassign the binding"). It is the fix for the recurring confusion
  that `&Object` is needed to *mutate* an object — it is not; it is needed to *replace*
  one.
- **No lifetime annotations.** A `&` is safe when the source outlives the binding; the
  borrow checker infers that from scope (rejecting `o = &mk()`, a `&` to a temporary),
  exactly as it already does for `&` *parameters* (the caller outlives the call). C38
  declined reference *types* with annotations; this is a binding *notation* the analysis
  resolves itself.
- **Aliasing is bounded, not a footgun.** The single owner still frees once; an alias is
  a tracked borrow in `deps` (the borrow system above). `&` makes the one non-obvious
  case — reassignment write-back — *explicit*, so no silent "did this propagate?"
  remains. The copy-vs-view question (#426 A/C) dissolves the other way from the original
  read: the view is the documented default, `&` covers the rest.
- **The payoff — the store-lifetime decisions collapse.** Once "is this binding a borrow
  of that store?" is one carried `deps` fact, the N places that re-derive ownership
  (#415's struct-field branch, the `a = x.v[i]` path, `has_ref_params`, the return-source
  set, the `block_result`/`ref_return` funnel, `reclaim_safe`) just read it. The
  store-lifetime bug class (Cluster A, #415, #426, the free-suppress + return-buffer
  leaks) is the *symptom* of computing it N times; landing the fact once closes the class.
- **Staging.** This is **loft's current behaviour minus one change**: make a non-`&`
  whole-binding reassignment a *local rebind* (today it overwrites the source in place),
  and let `&` carry the write-back — the existing `&vector<T>` param mechanism, now at
  bindings. Then ship the W4 redundant-`&` lint. **No value-semantics migration, no
  copy-on-write:** `a = vv[0]` stays a view, so #426 A/C are already correct.

## Why — the bug class is the symptom of an incomplete ownership system

The store-lifetime class (#405/#406/#409/#410 this cycle, cluster II of @PLN85) is
not N unrelated bugs. Each is a place where ownership was **re-derived by a codegen
heuristic** instead of **computed once as a fact**:

- `has_ref_params` at the call site, standing in for "does the return transfer
  ownership or borrow an arg?"
- `returned_var`'s single-`u16` structural walk, collapsing a `match`/`if` return
  to "no return var" → the returned arm buffers get freed.
- the return type left with an **empty / `"??"` dep** for `return v`, so a
  borrow-of-param return is indistinguishable from an owned return → the caller
  aliases the arg.
- **three separate return-handling paths** (`BlockTail` / `MidReturn` /
  native-forwarder) each re-deriving the same answer differently.

Every one is a missing or incomplete ownership fact. The class closes when the fact
is complete; it cannot be closed by adding more codegen conditions (that is what the
9 reverted @PLN85 attempts proved).

## Rust as the reference model

Rust is this beacon already realized — which is why it is the design reference:

- **Ownership and lifetimes are *type* facts**, computed once by the borrow
  checker. Drop insertion, move-vs-copy, and "may this value be returned" all
  *fall out* of those facts; codegen re-derives nothing.
- **Move by default; `Clone` is explicit.** A return value is *moved* to the
  caller, who becomes its sole owner. loft's `a = id(x)` aliasing bug is exactly
  the case Rust makes unrepresentable: returning `v` moves it out (or borrows it
  with a tracked lifetime); you cannot silently end with two owners.
- **Completeness is the whole game.** Rust computes ownership for *every* binding
  on *every* path — no "we didn't enumerate the `match` shape", no `"??"`. loft's
  bugs are precisely its incompletenesses.
- **It took years.** The borrow checker was a multi-release effort. loft's
  equivalent is a multi-cycle migration, not a patch.

The lesson, concretely: **do not bolt ownership onto codegen; make `deps` a
first-class, complete ownership analysis and let codegen read it.** The "thicket of
return paths" is what you get when ownership is re-derived per-shape; Rust avoids it
by having ONE place own the answer.

## loft today — a nascent borrow system

The pieces exist; they are incomplete:

- **Types carry `deps`** (typed `Deps`: `DepEntry::Attr(a)` | `DepEntry::CalleeFrame(w)`
  — see [DEPS_INVENTORY.md](DEPS_INVENTORY.md)). Semantics: empty dep ⇒ **owned**;
  `{Attr(a)}` ⇒ **borrows attribute a**.
- **The allocator has per-store liveness** (`free_bits` / `find_free_slot` in
  `src/database/allocation.rs`): an owned, live store is not recycled — the
  substrate a sound ownership system needs.
- **But the computation is partial** and supplemented by heuristics (above). The
  store-lifetime bug class is the catalogue of those gaps.

## The invariants the system must enforce (sound AND complete)

1. **Single owner.** Every heap store has exactly one owner at any moment.
2. **Move on return.** A returned heap value's ownership transfers to the caller's
   binding; the callee never frees what it transfers. If the return *borrows* a
   parameter, that is recorded on the return type (`{Attr(param)}`), and the caller
   copies to obtain its own store.
3. **Borrow tracking.** A value aliasing another (a param, field, or element)
   carries that source in its `deps`; the borrower is skip-free; the single owner
   frees once.
4. **Free placement is derived, not decided.** Free a local iff it owns its store
   and does not transfer it out — once, at scope exit. No per-site heuristic.
5. **Per binding, per path, complete.** Including every `match`/`if` arm — a set +
   reconcile, not a single-var structural walk.

When these hold, both backends translate the *same* facts, so interp and native
cannot diverge.

## The current holes — the migration backlog

| Hole | Symptom | The fact to complete |
|---|---|---|
| `returned_var` collapses `match`/`if` | returned arm buffers freed (cluster II / #405) | a return-**source set** (union of arms), not one var |
| ~~return dep empty for `return v`~~ ✅ CLOSED | `a = id(x)` aliased `x` | RESOLVED @PLN85: when the returned value is backed by a PARAMETER, the callee copies it into the return buffer (clear+append+return buffer, gated on `is_argument`) so the caller gets an owned copy — `control.rs` return promotion. Guard: `tests/scripts/85-store-lifetime-param-return-copy.loft`. (A copy-on-return, vs the originally-envisioned borrow-tag-then-caller-copies; same value-semantics outcome.) |
| ~~`has_ref_params` at the call site~~ ✅ CLOSED | adopt-vs-copy re-derived; vector returns alias (the `a = getv(x)` case of [probe 06](plans/85-store-lifetime-retirement/probes/06-field-read-adopt-vs-copy.loft)) | RESOLVED @PLN85 (A.3): the 11 `has_ref_params` decision sites (3 live — `scopes.rs`, `state/codegen.rs` ×2) now read ONE carried fact, `Definition::return_adopts_fresh_store()`: **return dep empty OR the `["??"]` one-buffer marker ⇒ adopt; any real attr index ⇒ copy**. This is STRICTLY BROADER than the A.4 `returns_borrowed_view()` (which only checks VISIBLE attrs): the adopt-vs-copy decision must ALSO copy a HIDDEN `ref_return` work-ref return (dep `["cv"]`, the caller-reused `__ref_N` buffer — over-unifying onto `returns_borrowed_view` regressed `143-plan51-cluster3` with a cross-iteration alias). The coarse `has_ref_params` over-approximated copy; the refinement flips copy→adopt ONLY for the genuinely-fresh case (`fn mk_from(seed) -> Box { Box{…} }`). `returns_borrowed_view()` stays as the *source-free-bit* fact (A.4, 3 sites). Guard: `tests/scripts/85-store-lifetime-return-ownership-adopt.loft` (+ `143` native leak-clean both backends). |
| ~~field read binds without an owner~~ ✅ CLOSED | `a = x.v` / `a = getv(x)` aliased the field's store | RESOLVED @PLN85 (#415): a STRUCT vector-field read now COPIES on bind (the @P292/@P394 bind-site branch admits a struct-field OpGetField) and on implicit-tail return (block_result copies a struct-field-of-argument return into `__retbuf`). Both narrowed to struct-field reads — a vector INDEX read (`vv[i]`) keeps its existing nested-stride path. Guard: `tests/scripts/85-store-lifetime-field-read-copy.loft`. (Copy-on-bind/return, the row-102 adopt-vs-copy fact instantiated for struct fields; a general dep-driven caller copy for arbitrary borrowing returns remains row 102's broader work.) |
| ~~value-`if`-return promotes ONE arm's buffer~~ ✅ CLOSED (a7) | `fn f(c) -> vector { if c { [..] } else { [..] } }` lost the true arm (interp read the sibling, native read empty — backends DIVERGED); struct/ref 3-arm `if` read the LAST arm on native | RESOLVED @PLN85 (a7): two facets of the row-100 `match`/`if` hole, for the `if` tail. (i) The function-return PROMOTION (`ref_return`/`text_return` renaming `__retbuf` to a body-tail local) is now gated to the GENUINE return context (`context == "return from block"`) — an `if`/`match` ARM is not the function tail, so no arm's `__vdb_N` is promoted; the fn-body tail delivers every arm into `__retbuf` (the `match` path). (ii) `unify_if_branches_work_refs` now collects EVERY arm's terminal work-ref across the whole if-tree and unifies them to one slot, so an `else if` chain (3+ arms) shares the return buffer like the 2-arm case. Guard: `tests/scripts/85-store-lifetime-if-return-owned-arms.loft`. (The `match` tail was already correct; this brings the `if` tail to parity. The row-100 "return-source set" remains the general framing.) |
| 3 return paths (`BlockTail`/`MidReturn`/native-forwarder) | each re-derives; fixes miss paths | funnel to ONE return-ownership computation. **A.2 partial:** the borrow-tail copy (a value backed by a visible param/field returned implicitly) now funnels to ONE helper `copy_borrow_tail_into_retbuf` shared by the struct-field tail (#415) and the whole-arg tail (`fn idv(v) -> vector { v }`), gated to `context == "return from block"` — both copy into `__retbuf` instead of `ref_return` recording a borrow dep. Guard `tests/scripts/85-store-lifetime-implicit-param-return-copy.loft`. The `if`-return buffer-model case (matrix a7) is **CLOSED (a7)** — see the row above. |
| `"??"` deps | unresolved ownership | compute the dep completely, no placeholder |

(The typed-`Deps` newtype work in [DEPS_INVENTORY.md](DEPS_INVENTORY.md) is the
groundwork already laid for this.)

## The migration discipline

Per [CODEGEN_METHOD.md](CODEGEN_METHOD.md): **one fact at a time, bottom-up,
working-vs-broken bytecode, study how Rust does it, validated on both backends.**
Replace heuristics with facts and *consolidate* the duplicated paths as you go —
each consolidation is a down payment on the beacon (one path, one fact). Expect
multiple cycles. Order by leverage: return-ownership first (it is the cluster-II
root and the most-reused decision), then nullability/layout/capture as their bug
shapes surface.

### Small steps are VITAL — not just tidy

Each fact is a **small** migration, and it must STAY small: a single, narrowly
scoped change that touches one decision and leaves the rest alone. This is the
load-bearing constraint, for two reasons:

- **It keeps the migrations small.** A small step is reviewable, testable on both
  backends in one rung, and revertible without collateral. A big-bang "rewrite the
  ownership system" is exactly the multi-hundred-line thrash the 9 reverted @PLN85
  attempts were — un-bisectable and unsafe.
- **It keeps the parser clean.** The danger when moving a fact INTO the
  type/parser layer is over-correcting — dumping a pile of new analysis into the
  parser and merely relocating the complexity (the caution in CODEGEN_METHOD § The
  balance). Small steps prevent that: each adds one well-defined fact, the
  heuristic it replaces is *deleted* in the same step, so the parser's net
  complexity stays flat or drops. The parser should get *cleaner* with each
  migration, never heavier.

**If a step is getting large, you have bundled facts — split it.** A migration that
can't be a small step is a sign the fact isn't isolated yet; find the one decision,
do that, and let the next step take the next decision. Net code should trend DOWN
(a fact replacing several heuristic branches), not up.

### Build on a mostly-working base — never break it to fix it

The migration takes time, and that time is the accepted cost of **never regressing
a mostly-working language**. Every small step LANDS green: both backends pass, the
suite is clean, the language works at least as well after as before (plus the one
fixed fact). We never take the language down for a multi-step rewrite; we never
leave a backend broken "to be fixed in the next step" (an interp fix that breaks
native compile is NOT landable — both green, or it doesn't land); we never trade a
working base for a half-built better one. **Time is fine; a broken intermediate is
not.** The base is always shippable, and each migration only ever adds correctness
on top of it. This is why the steps are small *and* sequential: a working language
at every commit is the substrate the whole effort stands on.

A fact is "done" when: it is computed once, completely (all shapes/paths); the
heuristic it replaces is deleted; codegen reads it in one place; the parser is no
heavier than before; and the rung's probe is green on both backends with no leak.

## Connections

- [CODEGEN_METHOD.md](CODEGEN_METHOD.md) — the *how* (the diagnostic + the rung discipline)
- [DEPS_INVENTORY.md](DEPS_INVENTORY.md) — the typed `Deps` substrate this builds on
- [LIFETIME.md](LIFETIME.md) — the current dep/scope model
- [STABILITY_HOTSPOTS.md](STABILITY_HOTSPOTS.md) — the forward risk register (ownership-by-shape-analysis is H-tier)
- [plans/85-store-lifetime-retirement/](plans/85-store-lifetime-retirement/) — the first application, incl. `type-ownership-design.md` (the return-ownership fact) and the bytecode rungs
