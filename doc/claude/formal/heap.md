<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/heap.md — small-step semantics for the store (strict)

**Catalogue:** @F3 (heap/store), @PLN85 (store-lifetime), @PLN89 (differential oracle).

> **Rules then deviations** (see [README](README.md)). This is the small-step evaluation
> relation for loft's **heap** — the store operations both backends must implement
> identically: allocation, field/element read + write, the whole-value COPY, and the free.
> It extends [operational.md](operational.md)'s scalar core with the heap `H`, and it is the
> written contract for exactly the part operational.md's D-op-1 named as *"unwritten … the
> interpreter remains their spec."*
>
> **Two docs, one heap, different questions.** [ownership.md](ownership.md) is the lifetime
> **CHECKER** — the static `deps` analysis that decides WHICH frees are sound (a value's
> `owns`/`borrows` fact). This doc is the **STEPS** — what `alloc`/`read`/`write`/`copy`/`free`
> actually DO to `H`, independent of whether a given free was wise. They meet at one theorem
> (`H-Sound` below): a `deps`-sound program never takes a step this semantics faults on (no
> use-after-free, no out-of-LIFO free, no aliased mutation of a copy). ownership.md proves the
> program only emits safe frees; this doc defines what a free is.

## The store model (the ground the rules stand on)

- **`H`** — the **heap**: a finite map `store_nr ⟼ Store`. `store_nr` is a `u16` slot index
  (`Stores.allocations`). Slot `0` is reserved for the interpreter's **evaluation stack**
  (`stack_store_at_zero`); it is never a freeable heap store.
- **`Store`** — a word-addressed byte region (`src/store.rs`): a header (signature,
  free-space index, record size) then content, with an intrusive free-block tree. It holds
  zero or more **records**, each a run of bytes at a record index.
- **`DbRef`** = `(store_nr: u16, rec: u32, pos: u32)` (`src/keys.rs`) — the **universal
  pointer** and a first-class runtime **value**: which store, which record, which byte
  offset within it. A field/element access is pointer arithmetic on `pos` — the byte offsets it
  adds are the [layout.md](layout.md) contract (heap.md gives the STEP, layout.md the FORMAT).
- **`nullref`** — `DbRef::NULL`, the reference null. It is the `E-Null` sentinel of the
  reference types (the same in-band discipline as `integer`'s `i64::MIN`).
- **locks** — a `Store` may be `read_only` (immutable: writes and frees fault) or
  `free_protected` (soft: frees fault, writes allowed) — set by the const store, the fn-call
  deep-copy bracket (a caller's argument is protected-from-free for the call), and worker
  borrows. Locks are part of the contract, not an implementation detail: a backend that
  ignores a lock steps differently.

## Notation

- `σ = ⟨ρ, H⟩` — the store/environment splits into the variable map `ρ` (operational.md's
  `σ(x)`) and the heap `H`. `⟨e, σ⟩ → ⟨e', σ'⟩` is one small step, as in operational.md.
- `r` ranges over `DbRef` values; `r ⊕ n` is `r` with `pos` advanced by byte offset `n`
  (same store + record). `H[r]` is the value stored at `r` (read at its width); `H[r ↦ v]`
  is `H` updated at `r`.
- `store(r)` is `r.store_nr`; `H ⊢ r live` means `store(r) ∈ dom(H)` and that store is not
  freed. `fresh(H)` is a `store_nr` not in `dom(H)`.
- `⊥` is the faulting configuration **only** where noted; per operational.md's C80, most
  "can't" cases are the value `null` + continue, **not** a halt (see `H-ReadNull`).

---

## Rules

### Heap values and the reference null

```
  (H-RefVal)   a DbRef r is a value (a normal form); it does not step.  (Named apart from
               `H-Ref` below, the `&`-bind ALIASING rule, which other docs cite by that name.)
  (H-RefNull)  nullref is the reference null — the per-type SENTINEL (E-Null) of a
               reference type.  Two configs that agree on the abstract reference (a live
               record, or nullref) MUST agree, however a backend encodes the sentinel.
```

**In words.** A heap reference is a finished value, just like an integer. `nullref` is a real
value (a reference that points at nothing), not a separate error state — the reference-typed
analogue of `integer`'s `i64::MIN` null.

### Allocation — a fresh store, zero-initialised

```
  (H-Alloc)    ⟨alloc τ, ⟨ρ, H⟩⟩ → ⟨r, ⟨ρ, H'⟩⟩
                 where s = fresh(H),  r = (s, 0, 0),
                       H' = H[s ↦ zeroed Store for τ]      (every field/element is its type's null/zero)
  (H-NewRec)   ⟨new-record r_v, ⟨ρ, H⟩⟩ → ⟨r_e, ⟨ρ, H'⟩⟩
                 a fresh record inside the vector/collection store r_v; r_e points at it,
                 its fields zero/null-initialised, the container's length grown by one.
```

**In words.** Allocating a struct/vector reserves a **fresh** store slot (`OpDatabase`) whose
content starts fully null/zero — a fresh `store_nr` distinct from every live store, so a new
value can never coincide with an existing one. Appending an element (`OpNewRecord` /
`OpFinishRecord`) claims a record inside the container's store. Construction is a pure
extension of `H`: it frees nothing and aliases nothing (this is why *constructing* a host
value is unrestricted under [capabilities.md](capabilities.md)'s `Cap-Own`).

### Read — through a live ref, or null-continue through nullref

```
  (H-Read)      ⟨read(r ⊕ n), ⟨ρ, H⟩⟩ → ⟨H[r ⊕ n], σ⟩            when H ⊢ r live
  (H-ReadNull)  ⟨read(nullref ⊕ n), σ⟩ → ⟨null, σ⟩               (deref of absent = null, CONTINUE)
  (H-Index)     ⟨read(r[i]), ⟨ρ, H⟩⟩ → ⟨H[r ⊕ stride·i], σ⟩       when 0 ≤ i < len(r)
                ⟨read(r[i]), σ⟩ → ⟨null, σ⟩                        when i out of bounds (OOB = null, CONTINUE)
```

**In words.** Reading a field/element is a read at the pointer's offset (`pos + field_offset`,
or `pos + stride·index`). Reading through **nullref**, or **out of bounds**, yields **null**
and execution **continues** — the same spreadsheet discipline as arithmetic (operational.md
`E-Uncomp`): an absent value degrades to null locally, it never halts the run.

### Write — update in place; null/lock faults are values or rejects, never wild writes

```
  (H-Write)      ⟨write(r ⊕ n, v), ⟨ρ, H⟩⟩ → ⟨v, ⟨ρ, H[r ⊕ n ↦ v]⟩⟩   when H ⊢ r live, store(r) writable
  (H-WriteNull)  ⟨write(nullref ⊕ n, v), σ⟩ → ⟨v, σ⟩                    (no store to update; a no-op step)
  (H-WriteOOB)   ⟨write(r[i], v), σ⟩ → ⟨v, σ⟩                           when i out of bounds (no-op, CONTINUE)
  (H-WriteLocked)  a write to a read_only store is a STATIC reject where provable, else a
                   runtime lock fault — never a silent successful write.
```

**In words.** A write updates the byte(s) at the target and yields the written value. A write
**through nullref** or **out of bounds** targets no live cell, so it is a no-op that continues
(it must never scribble on an arbitrary address — the null discipline extends to the write
side). A write to a **locked** store is refused (the `#lock` runtime guard / const store),
never silently applied. Crucially, a write's target ROOT decides whose state it touches: a
write whose root is a **parameter** mutates the caller's value; a write to a **local** touches
only that local's own store (see `H-Copy`) — the exact fact [capabilities.md](capabilities.md)'s
`Cap-Own`/raw-write admission rests on.

### Copy vs view — a whole-value / vector bind COPIES; a struct-typed projection is a VIEW

```
  (H-Copy)   ⟨x = r, ⟨ρ, H⟩⟩ → ⟨r', ⟨ρ[x ↦ r'], H'⟩⟩            when x = r is a PLAIN bind of a
               WHOLE VALUE — a local variable, or a VECTOR-typed projection (`fv = e.items`) —
               where r' = (fresh(H),0,0) and H'[store(r')] := deep-copy of the record graph at r.
               x and the source are then INDEPENDENT (C86, #415).
  (H-Ref)    ⟨x = &r, ⟨ρ, H⟩⟩ → ⟨r, ⟨ρ[x ↦ r], H⟩⟩              an EXPLICIT `&`-bind aliases: x
               shares r's backing (NO fresh store), so `x[0] = 99` mutates r (C77).  This is the
               vector twin of a parameter — both reach the source; only a plain bind copies.
  (H-View)   ⟨x = r, ⟨ρ, H⟩⟩ → ⟨r, ⟨ρ[x ↦ r], H⟩⟩               when r is a STRUCT-TYPED PROJECTION
               — a struct FIELD (`c = o.i`) or a struct ELEMENT of a vector (`s = v[0]`).  x is a
               VIEW: it aliases the place inside the container, so a write through x mutates the
               container (#426, ownership.md).  No store is allocated.
  (H-Materialise)  H-View holds only while the PLACE does.  Where the container is DISTURBED
               ([binding.md](binding.md) B-Disturb — a removal, a re-key, or a reassignment of the
               container) while x is still LIVE, the bind takes the H-Copy step instead: x gets a
               fresh store holding the value at the bind, and the author is told.  Writes through
               x then stop reaching the container.  A view whose last use PRECEDES the disturbance
               is unaffected and keeps aliasing (@PLN130 F2/F4/F8).  This is the plain-bind
               answer; an explicit `&` is DECLINED at compile time instead (B-Ref-Reshape),
               because a copy is not what it asked for.
```

**In words.** Whether a bind copies or aliases depends on **what is bound** — and the two backends
agree exactly (verified):

- **COPY** — a **plain** bind of a whole heap **value**: a local variable (`c = o`, `c = v`), or a
  **vector-typed** projection (`fv = e.items`). A fresh store is allocated and the graph
  duplicated, so the two are independent: `fv = e.items; fv[0] = 99` leaves `e.items[0] == 1`;
  `c = o; c.v = 9` leaves `o.v == 1`.
- **ALIAS** — an **explicit `&`-bind** (`r = &v`) binds a **live reference** ([C77](../DESIGN_DECISIONS.md#c77--binding-ownership-heap-aliases-by-default--binds-a-live-reference)),
  NOT a copy: `r = &v; r[0] = 99` makes `v[0] == 99` (verified both backends). This is why a `&`
  is written — to share the backing, not duplicate it. (A vector **parameter** likewise aliases
  the caller — see the invariant below.)
- **VIEW** — binding a **struct-typed projection**: a struct field (`c = o.i`) or a struct element
  of a vector (`s = v[0]`). `x` aliases the place, so a write through it mutates the container:
  `c = o.i; c.v = 9` makes `o.i.v == 9`; `s = v[0]; s.v = 9` makes `v[0].v == 9`.
- **…and a VIEW falls back to COPY when its place is destroyed under it** (`H-Materialise`). A
  container that is reshaped, re-keyed or reassigned while the view is still in use leaves the
  view pointing at nothing meaningful — a removal renumbers positions, so `c` starts naming a
  DIFFERENT element (measured: a read answered `44/444` where its element held `33/333`). The
  bind takes the copy step instead, and says so, so `c = v[0]; v.remove(2); c.n = 99` leaves
  `v[0].n == 11`. Order matters: `c = v[0]; c.n = 99; v.remove(2)` keeps the alias and lands the
  99, because the view is dead before the container changes.

This is the [DESIGN_DECISIONS.md C86](../DESIGN_DECISIONS.md) (`#415`) copy vs [ownership.md](ownership.md)
`#426` view boundary, and it is **exactly** the struct-vs-vector split
[capabilities.md](capabilities.md)'s raw-write rule (D-cap-3) already encodes: a **vector** local is
owned (copies, so writing it is `Cap-Own`), a **struct-typed** local may be a view of host data (so
its writes are host-gated). The invariant a raw write rests on is therefore *the target ROOT*: a
write reaches another binding's state only when its root is a **parameter**, an **explicit
`&`-reference bind** (`r = &v`, whose dep chain reaches the aliased source — possibly a
parameter), OR a **struct-typed view** of one — never a plain copy. The capability gate
(D-cap-3) enforces this by **following the vector's dep chain**: a local vector that aliases a
parameter (via `&`) is host, a genuinely-copied one is script-owned.

### Free — release a store, in LIFO order, never the stack, never twice

```
  (H-Free)       ⟨free(r), ⟨ρ, H⟩⟩ → ⟨(), ⟨ρ, H \ store(r)⟩⟩
                   when H ⊢ r live, store(r) is the MOST-RECENTLY-ALLOCATED live heap store,
                   store(r) ≠ 0 (not the eval stack), and store(r) not free_protected.
  (H-FreeNull)   ⟨free(nullref), σ⟩ → ⟨(), σ⟩                      (freeing null is a no-op)
  (H-FreeLIFO)   freeing a store that is NOT the current top of the allocation order is a
                 FAULT — the LIFO discipline (a store's lifetime nests within those allocated
                 before it).
  (H-FreeStack)  freeing store 0 (the evaluation stack) is a FAULT (#306): a stack-record ref
                 is never an owned heap store.
  (H-FreeTwice)  freeing an already-freed store is a FAULT (use-after-free / double-free).
```

**In words.** `free` releases a store slot and everything in it. It is disciplined: (1) **LIFO** —
you free stores in reverse allocation order, because a store's lifetime is nested inside the
stores allocated before it; (2) never the **stack** store (a `#306` bug is exactly a
stack-record ref mistaken for an owned heap store); (3) never **twice** (a double-free), and
never a store still reachable from a live binding (a use-after-free); (4) never a
`free_protected` store (a caller's argument during a call). `free(nullref)` is a harmless no-op.
Unlike a read/write, a bad free is a genuine **fault**, not a null-continue — it corrupts the
heap, so the discipline is a hard invariant, not a degradable value.

### The soundness bridge — a well-typed program never faults a free

```
  (H-Sound)   if a program is `deps`-SOUND (ownership.md O-Derived + O-Complete: every free
              it emits is on an OWNED store at its last use), then no execution reaches a
              faulting free — H-FreeLIFO / H-FreeStack / H-FreeTwice never fire, and no read
              observes a freed store.  The static checker discharges the dynamic invariant.
```

**In words.** These free rules describe what a free *does* and when it *would* corrupt the heap.
The promise that a real loft program never hits a corrupting free is **not** re-checked at
runtime — it is discharged statically by [ownership.md](ownership.md)'s `deps` checker: it only
emits a free on a store the value provably OWNS, at its last use, so LIFO holds, the stack is
never freed, and nothing freed is later read.

⚠ **That discharge is only as strong as the checker's register, and the register is not at
zero.** `ownership.md` is at `OPEN: 1` — `D-own-8`, *"a Join's ownership fact is true on one
path only"* — which is a PATH-COMPLETENESS gap, precisely the property `H-Sound` leans on. So
the free rules below are currently discharged by a checker with an open hole in the relevant
direction. Re-read that entry before treating a free fault here as impossible. This doc
defines the cliff; ownership.md proves the program walks the path beside it. The
`LOFT_POISON` harness is the empirical cross-check: it overwrites freed stores with a poison
pattern so any surviving `H-FreeTwice` / use-after-free surfaces as a corrupted read.

---

## Deviations

OPEN: **0** (this doc closes a *rules* gap, not a code deviation).

Writing these rules **shrinks** [operational.md](operational.md)'s D-op-1 — the heap/store
steps it named as *"unwritten … the interpreter remains their spec"* now have a written
contract (this file). What remains is the SAME meta-deviation, not a heap-specific one:

- **Conformance is differential, not definitional** — the heap steps here are enforced across
  the two backends by the @PLN89 **differential oracle** (D-op-1), whose corpus deliberately
  exercises the heap-heavy areas (collections, text, keyed collections, coroutines) where the
  interpreter's store and the native generator's `DbRef` ABI use the most different mechanisms.
  A program whose heap steps diverge is caught there. This doc does not add a new open row; it
  supplies the contract the oracle's heap-touching cases are read against.
- **The lifetime side has the strongest standing proof, and it is not complete.** The free
  discipline's soundness (`H-Sound`) rests on [ownership.md](ownership.md), whose register was
  at 0 when this line was written (2026-07-04) and is at **`OPEN: 1`** today: `D-own-8`, a
  Join's ownership fact holding on one path only. That is a path-completeness gap, and
  path-completeness is what `H-Sound` consumes — so the discharge is real but qualified.
  ⚠ A claim about another doc's register goes stale silently; re-read the register rather
  than this sentence.

---

## Conformance

The rules are checkable directly, and every check is a program both backends must agree on:

- **Copy vs alias vs view (`H-Copy` / `H-Ref` / `H-View`)** — proven both backends: a PLAIN
  whole-value / vector bind COPIES — `c = o; c.v = 9` ⇒ `o.v == 1`; `fv = e.items; fv[0] = 99` ⇒
  `e.items[0] == 1`. An EXPLICIT `&`-bind ALIASES — `r = &v; r[0] = 99` ⇒ `v[0] == 99` (a live
  reference, C77; the vector twin of a parameter). A STRUCT-typed projection is a VIEW —
  `c = o.i; c.v = 9` ⇒ `o.i.v == 9`; `s = v[0]; s.v = 9` ⇒ `v[0].v == 9` (a struct element of a
  vector). capabilities.md's raw-write rule encodes this: a plain-copied vector is owned, a
  `&`-aliased or parameter-rooted vector is host (D-cap-3 follows the dep chain), a struct is a
  possible host view.
- **Null/OOB continue (`H-ReadNull` / `H-Index`)** — reading a field of `nullref`, or `v[i]`
  with `i ≥ len(v)`, is **null** and the program continues; it never halts (operational.md
  C80, extended to the heap).
- **Parameter-root write escapes, local-root write does not (`H-Write` / `H-Copy`)** —
  `fn f(v: vector<integer>) { v[0] = 99 }` mutates the caller's vector (`orig[0] == 99`);
  binding first, `fn f(v) { c = v; c[0] = 99 }`, does not (`orig[0] == 1`). This IS the
  capabilities raw-write boundary.
- **Free discipline (`H-Free*`)** — the `LOFT_POISON` suite + the ownership fuzz gate are the
  standing falsifiers: any `H-FreeTwice` / use-after-free / out-of-LIFO free surfaces as a
  poisoned read or a leak-count mismatch. The register that guarantees they never fire is
  ownership.md (0 open).

D-op-1's falsifier applies here too: any program where the interpreter and `--native` diverge
on a heap step is the definitional error, and this doc is the definition it fails against.
