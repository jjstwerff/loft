<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->
# @PLN128 — Numeric-library bindings over `#c`

## Status

**Arcs A and B DONE; C recommended, not decided; D and E open.**  A purpose-built C library
reproducing each calling convention the numeric stack uses was bound through `#c` and run on
both backends; the matrix below is that measurement, not a reading of the ABI.  Most of the
boundary already works — including the one property nobody had tested — and the blockers are
not the ones previously assumed.

**The matrix was re-measured before any of it was written down, and one cell had to be
corrected: E6b.**  The retained-buffer case does not pass — it is a silent use-after-free on
both backends, and the earlier ✅ came from varying the wrong axis (allocation count instead
of whether the vector is used again).  That correction is the plan's most consequential
finding, because it removes FFTW as the cheap first consumer.  The working cells are now
guarantee probes in `tests/native.rs`.

**Issue:** [loft-lang/plans#128](https://github.com/loft-lang/plans/issues/128).

## Goal

loft binds the standard numeric stack — BLAS/LAPACK, FFTW, HDF5, GSL — through `#c`, with an
idiom a numeric author would accept.

## Effort + design

- **Effort:** M (arc A is XS; arc D is the M)
- **Design:** ~ (partial — the facts are settled, the contract decision is not)
- **Last touched:** 2026-08-10

## Composition matrix — Stage A (measured)

Re-measured 2026-08-10 against the committed `tests/fixtures/c_abi` library, with every
expected value computed by `lc_selftest.c` in C rather than read off a loft run.  (The
values differ from the original throwaway probe's because the fixture's inputs differ; what
matters is that each is an oracle, not a recording.)

| # | shape | `--interpret` | `--native` |
|---|---|---|---|
| E1 | read a double array — `vector<float>` → `(const double*, int64_t)` | ✅ 7750 | ✅ 7750 |
| E2 | **C writes back** through `double*` (the `daxpy` shape) | ✅ `[13 26 39]` | ✅ `[13 26 39]` |
| E4 | 1-element `vector<integer>` as a Fortran scalar `const int64_t*` | ✅ 42 | ✅ 42 |
| E5 | a scalar double in and out **by pointer** (arc B's idiom) | ✅ 5 | ✅ 5 |
| E6 | a **retained** buffer read after the call returned, the vector USED again later | ✅ | ✅ |
| E6b | the same, but the vector has **no later use** | ❌ **silent UAF** | ❌ **silent UAF** |
| E7 | **14 C argument slots** | ❌ refused (`0..=12`) | ✅ 1015 |
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

- **HDF5 / GSL** — pointer+integer APIs throughout; nothing in the matrix blocks them.
- **FFTW — NOT yet**, despite the pointer+integer API.  Its plan/execute split retains the
  caller's buffers between two calls, which is exactly the E6b use-after-free.  It needs
  either C-owned buffers held as opaque handles, or the retention declaration Q5 describes.
- **Fortran BLAS/LAPACK** — bindable behind a thin ANSI-C shim that collapses the
  by-reference argument list; the numeric core (arrays in, results written back) is proven.
- **CBLAS** — still blocked: `const double alpha` by value trips E3.

## Sub-arcs

| Item | Source | Status |
|---|---|---|
| **A** — write the matrix + the scalar-by-1-element-vector idiom into `PACKAGES.md` | this doc | **DONE** |
| **B** — fix the recommendation the refusal prints | this doc, Q3 | **DONE** |
| **C** — decide the backend capability contract | this doc, Q2 | Open — recommendation below |
| **D** — a shim generator for Fortran argument lists | this doc | Open |
| **E** — one numeric library bound end-to-end and dogfooded | this doc | Open |

**A (done).** `PACKAGES.md § Numeric libraries` carries the matrix, the
scalar-by-1-element-vector idiom, the two-slots-per-Fortran-scalar arithmetic that makes
`dgemm_` need a shim, and the retention hazard.  The mapping table's `float` row and the
12-parameter paragraph were both corrected: the latter claimed the ceiling was "checked on
every build", which `--native` does not honour.

**B (done).** All four refusal texts prescribed "a shim taking the bit pattern as an
integer", which loft cannot express — there is no float→bits conversion and `x as integer`
is a value cast (2.5 → 2, measured).  They now prescribe passing by POINTER, verified end to
end on both backends before the text was written.  The float→bits builtin was NOT added: the
pointer idiom already works and needs no new API surface.

**Noticed in B, not fixed (out of scope, worth its own slice).** One float declaration emits
**four** errors — `boundary_refusals` reports the return and the parameter, and `shape_of`
reports the same two again through a different path — so the author is told the same cure
four times for one mistake.  Deduplicating them means deciding which path owns the message,
which touches refusals beyond floats; it should not ride along on a text change.

**D and E are blocked, not skipped.** D is gated on the C decision above (it generates
against whatever contract C picks).  E has no target on the development box — no GSL, HDF5,
BLAS or LAPACK is installed, and no dev headers — and FFTW, the previously-nominated
cheapest target, is ruled out by E6b.

**Probes graduated.** The working cells are now
`native::numeric_array_shapes_cross_identically_on_both_backends`, against expected values
`lc_selftest.c` computes in C — agreement between two loft backends is not evidence that
either matches C.  E6b is deliberately NOT a test: asserting the current output would lock
in the use-after-free.

## Phase ordering

1. **A** first — the idiom that actually works is undocumented, which is why the earlier
   analysis reached for the bit-pattern shim instead.  Cheapest correction of the largest
   misunderstanding.
2. **B** — either make the recommended workaround expressible (a float→bits builtin) or
   change the message to name the idiom that works.  Depends on nothing.
3. **C** — the contract decision.  **D** and any ceiling work follow from it, so it gates
   them.
4. **D** — the shim generator, once C says what it is generating for.
5. **E** — **NOT FFTW** (its plan/execute split hits the E6b use-after-free).  HDF5 or GSL
   is the cheapest honest target now; BLAS level-3 is the one that proves D.

## Open design questions

1. **Scalar-by-pointer ergonomics.**  Wrapping every scalar in a 1-element vector works, but
   reads badly and costs a heap allocation per call.  A `&float` argument spelling, a
   documented `scalar()` helper, or docs alone — decide before writing the first real
   binding.
2. **The backend capability split.**  Does `#c` promise one capability on both backends, or
   may `--native` bind more?  loft's compatibility doctrine argues for one contract; the
   measured facts argue that uniformity costs `--native` capability it already has.  This is
   the plan's central decision and should be made explicitly.

   **Recommendation: ONE contract, and it is the interpreter's — 12 slots, both backends,
   refused at DECLARATION.**  Not implemented here, because it narrows what compiles today
   and that is the owner's call, not a side effect of documentation work.

   The reasoning is that the current split is not a capability, it is a **portability trap
   that fires late**.  A 13-slot binding compiles under `--native`, ships in a library, and
   fails only when some consumer interprets it — and the interpreter is not an optional
   backend: `loft debug` IS the interpreter, so a binding you cannot interpret is a binding
   you cannot debug, on someone else's machine, at the worst moment.  The refusal fires at
   the USE site rather than the declaration, so the library author never sees it at all;
   the cost lands entirely on a downstream consumer who did not write the declaration.

   That trade is bad in the direction that matters.  Uniformity costs `--native` a capability
   worth little on its own — a 13+-argument C function that must also be reachable from a
   shim-free binding is rare, and arc D's generator collapses exactly those argument lists
   anyway — while the split costs the language its "both backends run the same program"
   property, which everything else depends on.

   Two smaller things follow and do not need the decision made first:
   - **Q3 (float by value) resolves the same way** — keep it refused on both backends.
     Relaxing it on `--native` alone buys a narrow win for the same portability cost, and
     the pointer idiom (arc B) already covers the need.
   - **Q4 (raising the ceiling) becomes optional rather than blocking.**  If the ladder is
     ever extended it should move for BOTH backends at once, which is what makes "mechanical
     but unbounded" acceptable — the bound is chosen once, not per backend.

   If the opposite is chosen — `--native` may bind more — then the asymmetry needs to be
   declared in the package (a `requires-native` marker) and checked when the library is
   PUBLISHED, so the failure lands on the author rather than the consumer.  What must not
   survive is the current state, where the split is real, undeclared, and discovered late.
3. **Float by value.**  `--native` emits typed `extern "C"` and could pass a double in an SSE
   register today at no cost; the refusal is uniform only because the interpreter's
   trampolines are integer-class.  Relaxing it per-backend collides with Q2.
4. **Raising the interpreter ceiling.**  12 is a written-out ladder of transmute targets;
   extending it is mechanical but unbounded.  A shim-generation path may beat a longer
   ladder.
5. **The retained-buffer contract — ANSWERED, and the answer is a silent use-after-free.**
   E6b was re-measured and the earlier ✅ was wrong: it had varied the wrong axis.  The
   boundary is not the number of intervening allocations (0, 1, 8, 64, 512 and 2000 all
   pass) — it is **whether the loft vector has a later use at all**:

   ```loft
   a: vector<float> = [1.5, 2.25, 4.0];
   retain(a);                       // C keeps the pointer; `a` has no later use
   b: vector<float> = [100.0, 200.0, 400.0];
   reread();                        // reads `b`, not `a` — 700000 instead of 7750
   ```

   Measured identically on **both backends**: C reads another variable's data, with no
   fault and no diagnostic.  `retain(a)` does not extend `a`'s lifetime because loft cannot
   see that C stored the pointer, so `a` dies at its last loft-visible use.  This matches
   what `LoftCShape::PointerAndCount` already documents ("valid for the duration of the call
   only") — the contract was right and the measurement was wrong.

   **This removes FFTW as "the cheapest honest target" for arc E.**  `fftw_plan_dft` retains
   the buffers and `fftw_execute` reads them later, which is precisely the failing shape.
   A retaining C API cannot be bound directly until there is a way to DECLARE retention, so
   arc E should start from a non-retaining API (HDF5, GSL) or from C-owned buffers held as
   opaque handles.

   No diagnostic is proposed: the detectable condition ("a vector's last use is a `#c`
   call") is the *common correct* case — every read-only binding like `lc_i64_sum(v)` looks
   the same — so a warning would fire on the safe pattern and stay silent on the dangerous
   one.  The fix has to be a declaration (`#c` marking a parameter as retained), which is
   design work, not a lint.

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
