<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->
# @PLN128 — Numeric-library bindings over `#c`

## Status

**Open — no implementation, but the boundary is measured.**  A purpose-built C library
reproducing each calling convention the numeric stack uses was bound through `#c` and run on
both backends; the matrix below is that measurement, not a reading of the ABI.  Most of the
boundary already works — including the one property nobody had tested — and the blockers are
not the ones previously assumed.

**Issue:** [loft-lang/plans#128](https://github.com/loft-lang/plans/issues/128).

## Goal

loft binds the standard numeric stack — BLAS/LAPACK, FFTW, HDF5, GSL — through `#c`, with an
idiom a numeric author would accept.

## Effort + design

- **Effort:** M (arc A is XS; arc D is the M)
- **Design:** ~ (partial — the facts are settled, the contract decision is not)
- **Last touched:** 2026-08-04

## Composition matrix — Stage A (measured)

Every expected value hand-computed, every cell run on both backends.

| # | shape | `--interpret` | `--native` |
|---|---|---|---|
| E1 | read a double array — `vector<float>` → `(const double*, int64_t)` | ✅ 8000 | ✅ 8000 |
| E2 | **C writes back** through `double*` (the `daxpy` shape) | ✅ `[13 25 38]` | ✅ `[13 25 38]` |
| E4 | 1-element `vector<integer>` as a Fortran scalar `const int64_t*` | ✅ 42 | ✅ 42 |
| E5 | the prescribed bit-pattern shim (`int64_t` in, `int64_t` out) | ✅ | ✅ |
| E6 | a **retained** buffer read after the call returned | ✅ 8000 | ✅ 8000 |
| E6b | the same, after 2000 intervening vector/text allocations | ✅ 8000 | ✅ 8000 |
| E7 | **14 C argument slots** | ❌ refused (`0..=12`) | ✅ 140007 |
| E3 | a `double` **by value** | ❌ refused | ❌ refused |

**E2 is the headline.**  Every BLAS and LAPACK routine returns its result by writing through
a caller-supplied pointer.  Nobody had checked whether loft sees those writes.  It does, on
both backends — which is what makes binding the numeric stack worth doing at all.

## Three corrections to the earlier framing

1. **"Fortran-ABI BLAS binds today, unmodified" is false as stated.**  The reasoning —
   Fortran passes everything by reference, so no float travels by value — is sound, and
   E1/E2/E4 confirm the float half.  But `dgemm_` takes **13** by-reference arguments, and
   loft has no address-of for a scalar: each scalar must be wrapped in a 1-element vector,
   which costs **two** C slots (pointer + count), measured.  So `dgemm_` needs roughly 26
   slots against an interpreter ceiling of 12.  A shim is mandatory, not optional.

2. **The arity ceiling is interpreter-only.**  `--native` calls a genuine 14-slot C function
   and returns the correct hand-computed value; the interpreter refuses with a message that
   names `0..=12` explicitly.  The asymmetry is intended — it had simply never been measured.
   It means the two backends have **different binding capability**, which this plan must
   decide about rather than inherit.

3. **The prescribed float workaround is not expressible in loft.**  The refusal says to wrap
   the function in a shim "taking the bit pattern as an integer".  A real program holds a
   *computed* float, and loft has no float→bits conversion — `alpha as integer` is a **value**
   cast (2.5 → 2), measured.  The shim path works only for literals converted by hand
   offline.  It is also unnecessary: a scalar double already crosses correctly as a
   1-element `vector<float>`.  **The compiler recommends a remedy that is worse than the
   undocumented one that works.**

## What this makes possible now

- **FFTW / HDF5** — pointer+integer APIs throughout; nothing in the matrix blocks them.
- **Fortran BLAS/LAPACK** — bindable behind a thin ANSI-C shim that collapses the
  by-reference argument list; the numeric core (arrays in, results written back) is proven.
- **CBLAS** — still blocked: `const double alpha` by value trips E3.

## Sub-arcs

| Item | Source | Status |
|---|---|---|
| **A** — write the matrix + the scalar-by-1-element-vector idiom into `PACKAGES.md` | this doc | Open |
| **B** — fix the recommendation the refusal prints | this doc, Q3 | Open |
| **C** — decide the backend capability contract | this doc, Q2 | Open |
| **D** — a shim generator for Fortran argument lists | this doc | Open |
| **E** — one numeric library bound end-to-end and dogfooded | this doc | Open |

## Phase ordering

1. **A** first — the idiom that actually works is undocumented, which is why the earlier
   analysis reached for the bit-pattern shim instead.  Cheapest correction of the largest
   misunderstanding.
2. **B** — either make the recommended workaround expressible (a float→bits builtin) or
   change the message to name the idiom that works.  Depends on nothing.
3. **C** — the contract decision.  **D** and any ceiling work follow from it, so it gates
   them.
4. **D** — the shim generator, once C says what it is generating for.
5. **E** — FFTW is the cheapest honest target (pointer/integer API, no shim needed); BLAS
   level-3 is the one that proves D.

## Open design questions

1. **Scalar-by-pointer ergonomics.**  Wrapping every scalar in a 1-element vector works, but
   reads badly and costs a heap allocation per call.  A `&float` argument spelling, a
   documented `scalar()` helper, or docs alone — decide before writing the first real
   binding.
2. **The backend capability split.**  Does `#c` promise one capability on both backends, or
   may `--native` bind more?  loft's compatibility doctrine argues for one contract; the
   measured facts argue that uniformity costs `--native` capability it already has.  This is
   the plan's central decision and should be made explicitly.
3. **Float by value.**  `--native` emits typed `extern "C"` and could pass a double in an SSE
   register today at no cost; the refusal is uniform only because the interpreter's
   trampolines are integer-class.  Relaxing it per-backend collides with Q2.
4. **Raising the interpreter ceiling.**  12 is a written-out ladder of transmute targets;
   extending it is mechanical but unbounded.  A shim-generation path may beat a longer
   ladder.
5. **The retained-buffer contract.**  E6/E6b show the pointer survives — *while the loft
   vector is alive*.  What happens when it is not is untested, and that is exactly the
   FFTW-plan case.  Whatever the answer, it needs a documented pin rather than a measurement
   that happened to pass.

## Method note

The probes are throwaway (`sp.c` plus five `.loft` drivers).  Any that survive into a real
binding should graduate to `tests/scripts/` as guarantee probes — the **E2 write-back cell
especially**, since it is the property the whole plan rests on and nothing in the suite
currently pins it.

## Cross-arc dependencies

- **@PLN24** — the `#c` binding machinery this plan builds on; the arity ladder and the
  boundary refusals are its.
- **@PLN102** — the compatibility doctrine that Q2 has to answer to.

## See also

- [PACKAGES.md § Direct C binding — `#c`](../PACKAGES.md) — the binding contract; arc A's
  home.
- [`src/c_signature.rs`](../../../src/c_signature.rs) — `boundary_refusals`, the float refusal
  and its message.
- [`src/c_call.rs`](../../../src/c_call.rs) — the `0..=12` trampoline ladder.
- [`tests/fixtures/c_abi/`](../../../tests/fixtures/c_abi/) — the existing `#c` fixture and
  the `cc`-only build the probes were modelled on.
