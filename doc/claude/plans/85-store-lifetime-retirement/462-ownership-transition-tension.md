<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# #462 final blocker — the #316 ↔ #462 ownership-transition tension

The dominant #462 driver (`mon_one`/`mon_choose`, tp=76) is **one instance** of a shape
that pits the #316 fix against the #462 fix. Pinned via `LOFT_UAF_SRC` + a minimal repro
(`probes/over-free-sweep/M-462repro.loft`) and characterized by the `T1/T2/T3` probes.

## The shape

```loft
fn one(table, salt) -> M {
  pool = pool_of(table);          // a LOCAL vector
  chosen = m_none();              // (a) OWNED-init
  if np > 0 {
    chosen = pool[idx] ?? m_none(); // (b) conditional reassign with a nullable-unwrap VIEW
  }
  chosen                          // (c) RETURNED
}
```

`chosen = pool[idx] ?? m_none()` lowers (the `?? ` keeps the conservative `__nullable<M>`
result — operators.rs:1410) to `chosen = OpGetField(m, payload)` — a payload **sub-ref
VIEW** into `m` (= the local `pool`). `chosen` is then returned: the view escapes, `pool`
is freed at `one`'s exit, the return dangles, and `mon_choose`'s bind-copy reads the freed
store (SIGSEGV at slot-reuse scale).

## The tension (why a naïve fix oscillates)

| Fix | needs `chosen` to be… | because |
|---|---|---|
| **#316** (`151-i316-ownership-transition-free.loft`) | **OWNED** (no borrow dep) | so the **pre-Set free** of the old `m_none()` store fires at the reassign — else 1 store leaks per call |
| **#462** | recognised as a **VIEW** at the return | so the return is **copied** into `__retbuf` — else the sub-ref dangles |

A single static type for `chosen` cannot be both. The 2026-06-26 attempt (preserve the
nullable source's borrow deps through the unwrap, `expressions.rs` `nullable_to_dense_assign`)
made the return copy correctly (**M-462repro instrument went silent**) but **re-introduced
the #316 leak** (`wrap loft_suite` red) — the borrow dep suppressed the pre-Set free. Reverted.

## Scope (the T-probes, `probes/over-free-sweep/`)

| Probe | owned-init | cond view-reassign | returned | result |
|---|---|---|---|---|
| `T1-returned` | ✓ | ✓ | ✓ | 🔴 over-free + leak — the tension |
| `T2-notreturned` | ✓ | ✓ | ✗ | ✅ clean — only #316 applies |
| `T3-freshdecl` | ✗ (built as `if {view} else {m_none()}`) | — | ✓ | ✅ clean — the if-EXPRESSION return materialises |

All three conditions are required. **T3 is the fix beacon:** the expression form
(`chosen = if {view} else {m_none()}`) already materialises the view into an owned buffer
at the return; the imperative owned-init + reassign form does not.

## The fix direction (toward the defined semantics)

Make `chosen = pool[idx] ?? m_none()` (a nullable→dense reassignment of an **owning**
Reference local) **deep-copy the unwrapped payload into `chosen`'s owned store** — `OpDatabase`
+ `OpCopyRecord` from the payload sub-ref — instead of sub-ref-aliasing it. Then `chosen`
stays OWNED throughout: the #316 pre-Set free fires (it owns a store), AND the return is an
owned copy (#462 — no dangling view). This makes T1 behave like T3, satisfies both, and is
the value-semantics answer (assigning into an owning struct local copies). It is a codegen
change at the reassignment (interp `gen_set_first_ref_*` + native `output_set`), gated on the
target being an already-owning Reference local and the source a `__nullable<S>` unwrap — NOT
the whole-lifetime type change that broke #316.

## Acceptance signal

`LOFT_UAF_SRC` on `M-462repro.loft` must go **silent** AND native must be **leak-free**, AND
`151-i316` must stay green, AND the full suite + the crawler corpus
(`mon_one`/`mon_choose` tp=76 gone). NB the crawler has a SECOND driver — `sim_descend`'s
`ns` struct return (tp=194, `sim.loft:4121`) — a separate sub-shape to close after this one.
