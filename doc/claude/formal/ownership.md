<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/ownership.md — the `deps` ownership / borrow system (strict, aspirational)

**Catalogue:** @F21 (references `&T`), @I60 (deps / lifetime tracker) — Goal E. Roadmap: @PLN85, @PLN87.

> **Rules then deviations** (see [README](README.md)). ⚠️ **This area is aspirational by
> design.** The rules below are the model loft *steers toward* — they are mostly **not
> implemented yet**, so the deviation list is large and *is* the active migration
> (@PLN85 store-lifetime, @PLN87 the `&` law). The point of writing it now is direction:
> a clear target turns "another store-lifetime bug" into "a named hole in a known model".
>
> The rules are loft's borrow checker. **Rust is the reference model.** Beacon + rationale:
> [OWNERSHIP_MODEL.md](../OWNERSHIP_MODEL.md); the typed-`deps` design:
> [DEPS_INVENTORY.md](../DEPS_INVENTORY.md). This doc is the **checker** (lifetimes /
> free placement); the **surface** (`&τ`, reference-default) is [binding.md](binding.md).

## Notation

- **owner** — the binding or slot responsible for freeing a heap store. Exactly one at a
  time.
- **borrow / alias** — a value that *refers to* a store it does not own (a parameter, a
  field/element read, a `&τ` link). It must not free, and must not outlive its source.
- **`deps`** — the per-binding fact recording what it owns and what it borrows from. The
  one fact every store-lifetime decision reads. (Today a `Vec<u16>`; see D-own-3.)
- **transfer / move** — handing ownership to another binding (e.g. a return). The giver
  stops owning; it must not free what it moved.

---

## Rules

> The model is **sound** (no use-after-free, no double-free, no leak) **and complete**
> (computed for *every* binding, every path). The five invariants:

```
  (O-Owner)     SINGLE OWNER.  Every heap store has exactly one owner at any moment.
  (O-Move)      MOVE ON RETURN.  A returned heap value's ownership transfers to the
                caller's binding; the callee never frees what it transfers.  If the return
                *borrows* a parameter, the return type records it (`{Attr(param)}`) and the
                caller COPIES to obtain its own store.
  (O-Borrow)    BORROW TRACKING.  A value aliasing another (param / field / element / `&τ`)
                carries the source in its `deps`; the borrower is skip-free; the single
                owner frees once.
  (O-Derived)   FREE PLACEMENT IS DERIVED, NOT DECIDED.  Free a local iff it owns its store
                and does not transfer it out — once, at scope exit.  No per-site heuristic.
  (O-Complete)  PER BINDING, PER PATH, COMPLETE.  Every binding, including every `match`/`if`
                arm — a set-and-reconcile, not a single-variable structural walk.
```

**In words.** One thing owns each piece of heap, and it's the only thing that frees it.
When you return a heap value you *give it away* (the function stops owning it); if you
only return a view into an argument, the type says so and the caller makes its own copy.
Anything that just borrows is tracked but never frees. Crucially, *where* to free is
**computed** from these facts, not guessed per code-site — and it's computed for **every**
binding on **every** branch, not just the easy ones.

**This is an INTERNAL system — it never rejects a program.** loft has no user-facing borrow
checker; the user writes naively and the compiler always finds a valid lowering, copying when
it cannot prove an alias is safe ([OWNERSHIP_MODEL.md § Internal and invisible](../OWNERSHIP_MODEL.md)).
That makes **`O-Complete` the load-bearing invariant**: an incomplete fact is not a compile
error the user fixes — it is a miscompile or a leak. So the failure mode to fear here is
*incompleteness* (D-own-2), not just unsoundness — the analysis must be **total**.

### The mechanism — one fact, derived everywhere

```
  (O-Deps)      every store-lifetime codegen decision — free placement, adopt-vs-copy,
                move-vs-clone, drop — DERIVES MECHANICALLY from the single `deps` fact.
                If a decision is re-derived by a codegen condition, that is the bug.
  (O-NoDiverge) because both backends translate the SAME `deps` facts, the interpreter and
                `--native` cannot diverge.  (This is the soundness side of
                [operational.md](operational.md)'s shared contract: O-NoDiverge is *why*
                E-Op/E-Trap agree across backends.)
```

**In words.** `deps` is the single source of truth. Every "do I free / copy / move this
store?" question is *answered by reading `deps`*, never re-worked out in the code
generator. And because both backends read the same answer, they can't disagree — which is
exactly what makes the operational rules hold on native as well as interp.

---

## Deviations

OPEN: **5** — and unlike the other areas, here the deviations are the *bulk* of the
reality: the model is the beacon, the code is mid-migration.

### D-own-1 — ownership is re-derived per-site by codegen, not carried as one `deps` fact
- **Violates:** O-Derived / O-Deps
- **Where:** the store-lifetime bug class — `has_ref_params`, the return-source set, the
  free-suppress / return-buffer logic, etc. ([OWNERSHIP_MODEL.md § Why](../OWNERSHIP_MODEL.md)).
  Each fix added a codegen condition rather than completing a fact.
- **Effect:** the recurring store-lifetime bugs (Cluster A, #426, #429, …) — "N forests,
  one root". The class cannot be closed by more conditions.
- **Status:** OPEN — the active @PLN85 work.
- **Removal:** make every free/copy/move read `deps`; delete the per-site heuristics.

### D-own-2 — incomplete: not every binding/path has a computed ownership fact
- **Violates:** O-Complete
- **Where:** the row-100/102 holes — adopt-vs-copy for arbitrary borrowing returns; the
  general dep-driven caller copy. (The struct-field and value-`if`-return facets are
  CLOSED — #415, a7 — but the general framing is open: [OWNERSHIP_MODEL.md § holes](../OWNERSHIP_MODEL.md).)
- **Effect:** the uncovered paths fall back to a heuristic or a stopgap (D-own-4); a
  divergence hides until a test hits the path (operational.md D-op-2).
- **Status:** OPEN.
- **Removal:** compute ownership for every binding on every path (set-and-reconcile across
  `match`/`if` arms).

### D-own-3 — `deps` is an untyped `Vec<u16>` with overloaded markers
- **Violates:** O-Deps relying on `deps` as a sound, readable fact
- **Where:** the dep list is a `Vec<u16>` whose entries overload five meanings across two
  address spaces ([DEPS_INVENTORY.md](../DEPS_INVENTORY.md), H2).
- **Effect:** the "one fact" is hard to read correctly and easy to mis-derive — feeding
  D-own-1.
- **Status:** OPEN — the typed-`Deps` migration.
- **Removal:** a typed `Deps` representation so the fact is unambiguous.

### D-own-4 — stopgaps that contradict reference-default
- **Violates:** O-Borrow (a borrow should view, not copy) where a stopgap copies
- **Where:** #415 makes a STRUCT vector-field read COPY on bind — a narrowed store-lifetime
  stopgap, not the dep-driven view the end-state wants ([OWNERSHIP_MODEL.md:152](../OWNERSHIP_MODEL.md));
  it also blocks [binding.md D-bind-3](binding.md).
- **Effect:** correct-but-pessimistic (an extra copy) and inconsistent with reference-default.
- **Status:** OPEN — reverse once the dep-driven view (D-own-2) lands.
- **Removal:** struct-field reads become views via `deps`; delete the copy-on-bind branch.

### D-own-5 — the `&` borrow is built but not yet a `deps`-tracked borrow
- **Violates:** O-Deps for the explicit-link case
- **Where:** @PLN87 (PR#436, merged) **landed** the ladder L1–L6 — scalar/field/element/param
  references read + write through, leak-free ([binding.md](binding.md), verified). So `&`
  realises a live reference. What remains is the *checker* side: the `&τ` borrow's source is
  not yet carried as one `deps` fact the borrow checker reads — it shares the per-site
  ownership re-derivation of [D-own-1](#d-own-1).
- **Effect:** `&` works and is leak-free, but its lifetime is enforced by the existing
  store-lifetime machinery, not derived from a single `deps` borrow fact.
- **Status:** OPEN — folds into D-own-1 (one `deps` fact); the surface (binding.md) is closed.
- **Removal:** the `&τ` borrow carries its source in `deps` like any other borrow (O-Borrow),
  so free placement for a referenced lvalue derives from that fact.

---

## Conformance

This area's "falsifying programs" are the store-lifetime bugs themselves — each is a
program where the derived-free invariant (O-Derived) or completeness (O-Complete) fails
and a store leaks, double-frees, or a backend diverges. The area is **formal when OPEN
reaches 0**: when every store-lifetime decision is one `deps` read (O-Deps) over a complete,
typed fact, the bug class is closed by construction and `binding.md`/`types.md`'s
`deps`-fused rough spots (the `Deps`-in-`Type` fusion) resolve with it.
