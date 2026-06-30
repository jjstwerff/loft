<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Borrow-correctly returns — eliminate the field/whole-vector return copy (@PLN90 bucket 2 / @PLN85 P4)

The dominant **avoidable** copy (bucket 2) is the field / whole-vector return: a function
that returns a view of a parameter's heap data copies it into the caller's return buffer.
Eliminating it is also the @PLN85 **P4** fix — the broken borrowed-yield. This design pins
how before any codegen edit. References (current bytecode, both backends):
[field-COPIES-clean.txt](field-COPIES-clean.txt), [match-BORROWS-crashes.txt](match-BORROWS-crashes.txt).

## Current state (the two halves of the same shape)

| shape | what it emits today | result |
|---|---|---|
| `fn f(b: Box) -> vector { b.rows }` | `copy_borrow_tail_into_retbuf` **materialises a copy** of `b.rows` into `__retbuf` (`OpAppendVector(__retbuf, b.rows)`). Return `["__retbuf"]` (owned). | clean, **but a copy** (the avoidable copy) |
| `match e { Filled { items } => { items } }` | returns the **alias** `_mv_items_1 = OpGetField(e,4)` directly, *through* the owned `__retbuf` buffer ABI (buffer-in, alias-out). Return `["__retbuf","e"]` (borrows e). | **P4 derail** — the buffer-ABI/alias mismatch |

So loft already has both halves: it knows how to copy (f) and how to alias (g) — it just
does the wrong one for each, and the alias path is mis-wired.

## The invariant

> **A function whose tail returns a borrowed view of a parameter returns the aliased
> `DbRef` directly — no return buffer, no copy. The COPY moves to the CALLER, emitted only
> at a call site where the borrow would be unsound (the aliased subject does not outlive
> the result, or the result must be owned).**

The copy does not disappear; it **moves from the callee (always) to the caller (only when
needed)**. Most call sites keep the subject alive across the result → no copy. That is the
elimination.

## Why the decision is caller-side (the load-bearing fact)

A borrowed return is sound **iff the subject outlives the result's last use** — and that is
known only at the **call site**, not in the callee:

- `c = Filled { items: ti }; a = g(c)` — `c` is a named local that outlives `a` → **borrow**, no copy.
- `a = g(Filled { items: ti })` — the `Filled` is a temporary, freed after the call → the
  alias would dangle → the caller **must materialise** a copy.

This is exactly the `deps` / `ownership_of` lifetime fact (@PLN85, OWNERSHIP_MODEL.md). The
callee cannot decide alone; the caller reads whether the subject out-lives the result.

## Re-assertion sites & the brittleness count

The callee emits the borrow once (N=1). The **caller** must decide borrow-vs-materialise at
**every call site** of a borrow-returning function (N = call sites). Omitting the
materialise where it is needed is a **silent UAF**, not a compile error — so `N × silence`
is the hazard. Cure: the decision must be a single mechanical read of the `deps` fact
("subject out-lives result?") at the one caller-side chokepoint that lowers a call, not a
heuristic re-derived per site. If `ownership_of` already carries the fact, N collapses to a
read.

## Failure paths (enumerate before coding)

| # | failure | guard |
|---|---|---|
| F1 | temporary subject (`g(Filled{…})`) → alias dangles | caller materialises when the subject does not out-live the result |
| F2 | result escapes as owned (returned further / stored long-lived / mutated) | caller materialises (the result must own its store) |
| F3 | the callee still gets a `__retbuf` and returns an alias (today's P4 mismatch) | borrowed-view returns drop the buffer ABI — return the alias as a plain `DbRef` |
| F4 | caller frees the borrowed result → over-free (the subject owns it) | the result's `deps` mark it borrowed → caller skip-frees (already true for the match-arm caller `a["c"]`) |
| F5 | the borrow path regresses an owned shape that *should* copy | gate behind the existing `LOFT_JOIN_OWN` (or a new flag); suite byte-identical off |

## Implementation slices (each matrix-validated on BOTH backends, gated)

1. **Callee ABI** — a borrowed-view return (tail deps depend on a param, not on a fresh
   local/`__retbuf`) returns the aliased `DbRef`; drop the `__retbuf` fill. This is the
   direct-alias-return ABI; it makes `g` consistent (fixes the P4 derail) and stops `f`
   copying. Capture the target bytecode (hand-write the clean alias-return) beside the
   current ones first (loft-codegen gate).
2. **Caller materialise-on-demand** — at each call of a borrow-returning fn, read the
   `deps` fact; if the subject does not out-live the result (F1) or the result escapes-owned
   (F2), emit the materialising copy at the call site; else bind the borrow + skip-free (F4).
3. **Retire the callee copy** — `copy_borrow_tail_into_retbuf` becomes the *caller-side*
   materialiser from slice 2; the always-copy callee path is removed.

Slices 1 and 2 are **coupled** — slice 1 alone makes temporary-subject calls UAF (F1), so
it ships gated until slice 2 lands. Validate with the boundary matrix: subject-outlives
(borrow, no copy, len+value+leak) × temporary-subject (materialise) × result-escapes
(materialise) × both backends.

## Boundary matrix (current behaviour captured, gate off)

Cells: [cell-named.loft](cell-named.loft) (subject out-lives → must borrow),
[cell-temp.loft](cell-temp.loft) (temporary subject → must materialise),
[cell-escape.loft](cell-escape.loft) (result escapes through `h` → propagate/materialise).

| cell | interp (gate off) | native (gate off) | target (both) |
|---|---|---|---|
| named | **panics (P4)** | clean (valid borrow) | clean borrow, **no copy** |
| temp | **panics (P4)** | "ok" — but a **latent UAF** (the temporary's store not yet reused) | caller **materialises** → clean |
| escape | **panics (P4)** | "ok" — latent UAF | propagate the borrow, or materialise |

So interp's P4 crash masks the real correctness question (temp/escape are UAF on native,
silently). Slice 1+2 must make all six cells correct — value + length + leak (`LOFT_POISON`
to force the temp UAF loud), both backends.

## Slice-1 edit points + the two-pass complication (found while prepping)

- The `__retbuf` buffer attribute is created in **`src/parser/definitions.rs:992-1015`**,
  driven by the **return TYPE** (a heap return → a buffer), *before* the body is parsed.
  But **borrow-ness is a BODY property** (does the tail alias a param?) known only later —
  so the buffer cannot simply be suppressed at creation time. Options: (a) a first-pass
  body scan to predict a borrowed return, then suppress on the second pass; (b) retire the
  buffer attribute (`retire_argument`, already used by `ref_return`) once the return is
  known borrowed; (c) keep the buffer but make the borrowed-return path through it
  CONSISTENT (return the alias, fix the P4 frame/discard accounting) — a smaller fix that
  removes the *big* copy (`f`'s `copy_borrow_tail_into_retbuf` → return the alias) and the
  crash, leaving only a tiny wasted empty-buffer alloc to optimise later.
- The tail copy/alias choice is in **`src/parser/control.rs`** — `ref_return`,
  `copy_borrow_tail_into_retbuf` (the `f` copy to make conditional), `materialize_return_into`.
- Reuse the **`LOFT_JOIN_OWN`** gate (already wraps the @PLN85 borrowed-yield work).
- **No safe partial:** slice 1 (callee borrows) without slice 2 (caller materialises on a
  temporary/escape) is a UAF on the temp/escape cells — so the two ship together, gated;
  the suite stays byte-identical with the gate off.

Next concrete step: plot the target alias-return bytecode for the `named` cell (hand-write
the clean `DbRef`-return + the caller binding), prove it standalone, then implement
option (c) (smallest) behind the gate and walk the matrix.

## Connection back

This is the `Borrow`-set growth the @PLN90 north-star is about: field-return moves from
bucket 2 (avoidable, warned) to bucket 1 (auto-eliminated). It is simultaneously @PLN85
P4's resolution — the same borrowed-yield, compiled correctly as a borrow instead of
crashing or being copied. See [../phase1-inventory.md](../phase1-inventory.md) and the
[@PLN85 handoff](../../85-store-lifetime-retirement/NEXT-SESSION-match-return.md).
