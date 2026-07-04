<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN85 / D-own-2 — complete the ownership fact for every binding/path

The completeness deviation ([formal/ownership.md D-own-2](../../formal/ownership.md)):
*not every binding/path has a computed ownership fact; uncovered paths fall back to
a heuristic/stopgap, and a divergence hides until a test hits the path.*  This doc
records the 2026-07-04 measurement + the one concrete live bug it surfaced.

## Measurement 1 — the free-side classifier vs the oracle (latent, not a bug)

Instrumented every `scan_set` #316 site to log where `ref_rhs_ownership` (free-side)
and the `ownership_of` oracle would drive a DIFFERENT `owned_refs` action, swept all
359 `tests/scripts`: **8997 divergences, 100% one-directional** —
`free=Unknown(don't-track)` vs `oracle=Owned(track)`, ALL on compiler-gen temps
(`_elm_N`/`__vdb_N`/`__ref_N`).  ZERO dangerous-direction divergences (the free side
never says Owned where the oracle says Borrowed).  So the free-side classifier is
*incomplete but SAFE*: it conservatively leaves owned temps untracked (freed at scope
exit instead of at the #316 transition), and those temps don't reach the transition-
free's firing condition anyway.  No live bug; unifying the two onto one three-valued
fact (Owned / Borrowed / **Unknown**, each consumer rounding Unknown its own safe
way) is the clean completion — a semantic oracle change, @PLN90's charter.

## Measurement 2 — a LIVE bug: struct whole-value copy-return leaks on native

Probing arbitrary borrowing returns surfaced a real one
([`probes/d-own-2/`](probes/d-own-2/)):

```loft
fn idcopy(x: Box) -> Box { r = x; r }          // struct — LEAKS Box×1 on native
fn vcopy(x: vector<int>) -> vector<int> { r = x; r }   // vector — CLEAN both backends
fn choose(cond, x: Box) -> Box { r = x; if cond { r = Box{..}; } r }  // the JOIN face — LEAKS
```

- **Symptom:** native leaks the returned store; interp is clean (a D-op-2 backend
  divergence).  `idcopy`'s return type is `Box["r", "x"]`; `vcopy`'s is
  `vector<int>["??"]` (owned).  The struct return carries the VISIBLE param `x`, so
  the caller's `returns_borrowed_view()` reads *true* → it treats the result as a
  borrow of `x` → never frees the owned copy.
- **Root (exhaustively traced with `LOFT_TRACE_RR`):** a CROSS-PASS staleness.
  Per C86, `r = x` COPIES (confirmed: mutating `r` doesn't touch `x`; native emits
  `OpDatabase`+`OpCopyRecord`), so `r` OWNS its store and must not carry `x`.  The
  C86 whole-value copy dep-strip is implemented for VECTORS (`classify_vec_bind` →
  `make_independent`, `parser/expressions.rs`) but there is NO struct equivalent —
  AND the var-copy strip is deliberately pass-2-only (to avoid the vector elm-var
  counter drift).  So in **pass 1** `r`'s dep is still `[x]`; `ref_return` pulls `x`
  into the return dep (`v=x verdict=MergeAttr{a:0}`).  In **pass 2** `r` is stripped
  to owned (`Deps{items:[]}`), but `ref_return` seeds `dep = cur.clone()` from the
  pass-1 return type — carrying the stale `x` (`ret=Reference(621, [1, 0])`).  The
  vector path does not hit this (its `ref_return` handling yields the `["??"]`
  one-buffer owned marker); the struct/Reference path does.
- **The fact to complete (D-own-2):** `r = x` on a struct is a COPY → the return
  OWNS `r`'s store and does NOT borrow `x`.  Two candidate fix loci, both needing
  care in the store-lifetime foundation (hence deferred, not rushed):
  1. Give the STRUCT whole-value copy the same C86 dep-strip vectors get, and make it
     run in pass 1 too (a Reference bind has no vector elm-var counter, so the
     pass-1 drift the vector comment guards against may not apply — VERIFY).
  2. In `ref_return`, do not carry a VISIBLE-attr return dep that NO pass-2 return
     source justifies (guard against H5 — pass-1 facts are otherwise frozen).
- **Guard:** `probes/d-own-2/{struct-copy-return-leak,join-copy-return-leak}.loft`
  (leak on native) + `vector-copy-return-clean.loft` (the clean vector control).
  NOT graduated to `tests/scripts/` yet — the wrap leak-gate would go red until the
  fix lands.

## Status

- [x] Measured the free-side/oracle incompleteness — latent + safe (one-directional).
- [x] Root-caused the live struct-copy-return native leak (cross-pass stale `x`).
- [ ] Fix (locus 1 or 2 above) — the concrete D-own-2 slice; careful pass-timing +
      both-backends + oracle validation required.  Folds into @PLN90.
