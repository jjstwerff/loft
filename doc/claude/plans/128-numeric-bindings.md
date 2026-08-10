<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->
# @PLN128 — Numeric-library bindings over `#c`

## Status

**Arcs A, B, C and D DONE; E blocked on this box.**  A purpose-built C library
reproducing each calling convention the numeric stack uses was bound through `#c` and run on
both backends; the matrix below is that measurement, not a reading of the ABI.

**Arc D did not need a shim generator — it needed the boundary corrected.**  The fixture's C
functions were all written to loft's pointer-and-count shape, which held fixed the one axis
that decides whether a numeric library binds at all.  Measured against a *genuine* `dgemm_`
signature (thirteen bare pointers, no counts), **no Fortran routine was bindable at any arity
ceiling**: the honest declaration was refused for arity, and the shape loft accepted delivered
each count where the callee expected the next pointer — SIGSEGV on the interpreter, nothing
under `--native`.  The fix is that **the C signature decides whether a `vector` carries a
count** (C107).  `dgemm_` now costs 13 slots rather than the 26 this plan asserted, binds
directly, and copies nothing.

**Two earlier claims were wrong and are corrected below**: "`dgemm_` binds without an
argument-collapsing shim" (it did not bind at all), and "a Fortran argument list costs two C
slots per scalar" (it costs one).  Both came from measuring a fixture built to loft's own
shape — the benchmark held the axis fixed for free.

**E6b remains the plan's most consequential finding.**  The retained-buffer case is a silent
use-after-free on both backends; the earlier ✅ came from varying allocation count instead of
whether the vector is used again.  It removes FFTW as the cheap first consumer.  The working
cells are guarantee probes in `tests/native.rs`.

**Issue:** [loft-lang/plans#128](https://github.com/loft-lang/plans/issues/128).

## Goal

loft binds the standard numeric stack — BLAS/LAPACK, FFTW, HDF5, GSL — through `#c`, with an
idiom a numeric author would accept.

## Effort + design

- **Effort:** M (arc A is XS; arc D was the M, and turned out to be S once measured — a
  boundary correction rather than the generator it was scoped as)
- **Design:** ✓ (C106 fixes the arity contract, C107 the count contract)
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
| E7 | **14 C argument slots** | ✅ 1015 | ✅ 1015 |
| E7b | **33 C argument slots** (past the new ceiling) | ❌ refused | ❌ refused |
| E3 | a `double` **by value** | ❌ refused | ❌ refused |
| **F1** | a **genuine Fortran routine** — 5 bare pointers, no counts (`daxpby_`) | ✅ 1003 2004.5 4008 | ✅ same |
| **F2** | **`dgemm_` at full width** — 13 by-reference arguments, 13 slots | ✅ 1046 2068 3062 4092 | ✅ same |
| **F3** | the two `char *` flags land where the callee reads them | ✅ −1 on `'T'` | ✅ same |
| **F4** | counted and bare vectors **mixed in one signature** | ✅ 228000 | ✅ same |

The F-rows are arc D, and they were all ❌ before it: F1/F2 refused for arity when declared
honestly, and SIGSEGV when declared the way loft insisted on.  Expected values come from
`lc_selftest.c`, which computes them in C.

**E2 is the headline.**  Every BLAS and LAPACK routine returns its result by writing through
a caller-supplied pointer.  Nobody had checked whether loft sees those writes.  It does, on
both backends — which is what makes binding the numeric stack worth doing at all.

## Three corrections to the earlier framing

1. **"Fortran-ABI BLAS binds today, unmodified" was false as stated — and the arithmetic
   this plan replaced it with was false too.**  The reasoning —
   Fortran passes everything by reference, so no float travels by value — is sound, and
   E1/E2/E4 confirm the float half.  This plan then asserted that each by-reference scalar
   costs **two** C slots, so `dgemm_` needs 26, and that arc C's ceiling of 32 cleared it.

   **Both halves of that were wrong, and arc D is the correction.**  A count is not
   something loft adds to a Fortran call — it is something the Fortran routine does not
   *take*.  Declared honestly, `dgemm_` was refused ("takes 13 parameter(s), the loft
   declaration needs 26"); declared with counts, the counts landed where the callee expected
   pointers and it crashed.  The ceiling was never the binding constraint.  Since C107 a
   `vector` carries a count only where the C signature has one, so `dgemm_` costs **13**
   slots, binds directly, and LAPACK's 20+-argument drivers fit under 32 as well.

2. **The arity ceiling was interpreter-only — FIXED in arc C.**  `--native` called a genuine
   14-slot C function and returned the correct hand-computed value while the interpreter
   refused the same declaration naming `0..=12`.  The two backends had **different binding
   capability**.  They no longer do: the ladder was extended to 32 and both are held to it.

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
- **Fortran BLAS and LAPACK** — bindable DIRECTLY since arc D.  One slot per by-reference
  argument, so `dgemm_` costs 13 and even the 20+-argument drivers fit under 32.  The
  numeric core (arrays in, results written back, nothing copied at the boundary) is proven
  against a C oracle on both backends.
- **CBLAS** — still blocked: `const double alpha` by value trips E3.

## Sub-arcs

| Item | Source | Status |
|---|---|---|
| **A** — write the matrix + the scalar-by-1-element-vector idiom into `PACKAGES.md` | this doc | **DONE** |
| **B** — fix the recommendation the refusal prints | this doc, Q3 | **DONE** |
| **C** — decide the backend capability contract | this doc, Q2 | **DONE** |
| **D** — make Fortran argument lists bindable | this doc | **DONE** |
| **E** — one numeric library bound end-to-end and dogfooded | this doc | Blocked — no target installed |

**A (done).** `PACKAGES.md § Numeric libraries` carries the matrix, the
scalar-by-1-element-vector idiom, the two-slots-per-Fortran-scalar arithmetic that sizes the
ceiling, and the retention hazard.  The mapping table's `float` row and the arity paragraph
were both corrected; the latter claimed the ceiling was "checked on every build", which
`--native` did not honour until arc C.

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

**C (done).** See the decision under Q2 below.  One contract at `MAX_C_ARITY = 32`, enforced
on both backends — at the declaration in owned code, and at every call site.

**D (done) — and it was not the arc that was written down here.**  The scoped work was a
shim generator for the routines that overflow 32 slots.  Measuring first showed the premise
was wrong twice: *every* Fortran routine needed help, not just the large ones, and the help
needed was not an argument-collapsing shim.

The fixture is what hid it.  `lc_types.c` was written to loft's pointer-and-count shape, so
every arity cell in the matrix above ran against a C library that took counts — the axis
that decides whether a numeric library binds was held fixed for free.  A purpose-built
`dgemm_` with the real signature answered in one run: honest declaration refused for arity,
loft-shaped declaration SIGSEGV.

Three routes were measured before choosing:

| route | works | why not the default |
|---|---|---|
| C-owned buffers as opaque `integer` handles | ✅ both backends, oracle-matched | copies every array in and out — 8 MB per call on a 1000×1000 matrix, which is what calling BLAS was for.  Stays the right answer for a **retaining** API (it also dodges E6b) |
| a generated ANSI-C shim per routine | (not built) | puts a C toolchain in every numeric package's build, to work around a boundary that can just be correct |
| **the signature decides the count** | ✅ chosen | zero-copy, no new surface, no toolchain; a loosening, so nothing that compiled stopped |

`c_signature::plan` is the one home for the assignment; the declaration check, the
interpreter's `dispatch` and the `--native` emission all read it.  The counted paths are
byte-identical before and after (`loft introspect` diff), and the guard was verified by
reintroducing the bug on each backend separately — the interpreter SIGSEGVs and `--native`
fails to compile, and the test catches both.

**E is blocked on this box, not on loft.**  No GSL, HDF5, BLAS or LAPACK is installed, no dev
headers, and no passwordless `sudo` to add one; FFTW, the previously-nominated cheapest
target, is ruled out by E6b regardless.  What arc E needs now is a machine with
`libopenblas-dev` or `libgsl-dev` — the language side is proven against a C oracle, so the
remaining work is dogfooding a real library rather than fixing the boundary.

**Probes graduated.** The working cells are now
`native::numeric_array_shapes_cross_identically_on_both_backends`, against expected values
`lc_selftest.c` computes in C — agreement between two loft backends is not evidence that
either matches C.  The fixture gained `lc_daxpby_`, `lc_dgemm_` and `lc_split_`, which are
the first functions in it written to a **foreign** convention rather than to loft's; that is
the point of them, and the reason the earlier matrix could not see arc D.  E6b is
deliberately NOT a test: asserting the current output would lock in the use-after-free.

**One defect the fixture surfaced, fixed here.**  `advice[too-many-parameters]` fired on
every Fortran binding (`dgemm_` takes 13) and both cures it names are impossible at a `#c`
boundary: a struct cannot cross at all, and a default does not change the arity the C
signature declares.  It no longer fires on a `#c` declaration — the parameter list is the C
function's, not the author's.

## Phase ordering

1. **A** first — the idiom that actually works is undocumented, which is why the earlier
   analysis reached for the bit-pattern shim instead.  Cheapest correction of the largest
   misunderstanding.
2. **B** — either make the recommended workaround expressible (a float→bits builtin) or
   change the message to name the idiom that works.  Depends on nothing.
3. **C** — the contract decision.  **D** and any ceiling work follow from it, so it gates
   them.
4. **D** — done, and not as a shim generator: the boundary was corrected instead.  The
   ordering assumption that C gated D held, but for the opposite reason — C's arithmetic was
   what pointed at the wrong fix.
5. **E** — **NOT FFTW** (its plan/execute split hits the E6b use-after-free).  BLAS or
   LAPACK is now the cheapest honest target, since arc D binds them with no shim; GSL and
   HDF5 remain fine.  Needs a machine with the dev package installed.

## Open design questions

1. **Scalar-by-pointer ergonomics — still open, and now the only ergonomic gap left.**
   Wrapping every scalar in a 1-element vector works and costs one slot since arc D, but it
   still reads badly and still costs a heap allocation per call.  A `&float` argument
   spelling, a documented `scalar()` helper, or docs alone.

   **One route is closed:** letting a plain `integer`/`float` bind to a C pointer type and
   meaning "pass its address".  `integer` against a C pointer already means the HANDLE
   convention (pass the value), which `lc_open`/`lc_read` depend on in both directions, so
   the same spelling would mean two things and no runtime signal separates them.
2. **The backend capability split.**  Does `#c` promise one capability on both backends, or
   may `--native` bind more?  loft's compatibility doctrine argues for one contract; the
   measured facts argue that uniformity costs `--native` capability it already has.  This is
   the plan's central decision and should be made explicitly.

   **DECIDED (arc C, implemented): ONE contract at `MAX_C_ARITY = 32`, enforced on BOTH
   backends — at the declaration in owned code, and at every call site.**

   The recommendation first written here was *one contract at the interpreter's 12*.  That
   was the right shape and the wrong number, and COMPATIBILITY.md is what corrected it.  Its
   rule for the error surface: dropping an error is always safe, adding one is a break, and
   pre-freeze the disposition inverts to "be strict now" — but *"the first resolution of a
   would-be-error is a rewrite to correct function, not an error.  Erroring is the narrower
   choice, reserved for what cannot be given a sane defined behavior."*

   Unifying at 12 would have narrowed what compiles today for no reason other than that the
   ladder was short.  A functioning rewrite existed: the trampoline ladder is one
   `extern "C" fn(u64 × N) -> u64` per rung, purely mechanical, so raising it is LOOSENING —
   which the promise permits unconditionally — and it makes the two backends agree without
   taking anything away.

   **Why 32 rather than "as high as possible".**  A ladder cannot be unbounded, so some
   number is refused on both backends and that half is a genuine tightening (pre-freeze, and
   the last-chance-to-add the doctrine describes).  32 is sized off the worst case this plan
   actually names: `dgemm_`'s 13 by-reference arguments cost 26 slots, so 32 clears it with
   margin.  Past that a shim is the honest answer, which is what arc D is for.

   **What it cost and what it bought.**  Cost: a `#c` binding of 33+ C slots, which compiled
   under `--native` before, is now refused.  No real C API is anywhere near that.  Bought:
   the two backends run the same program; `loft debug` can reach every binding that builds;
   `dgemm_` binds with no shim at all; and the failure now lands on the author, at the
   declaration, instead of on a downstream consumer at a call site.

   **Enforcement is deliberately two-pass**, mirroring `superseded_fold_diagnostics`:
   declarations are checked only in code you OWN (stdlib or entry project), so merely loading
   a dependency that declares an over-ceiling binding you never call does not fail your build
   — a consumer cannot edit someone else's declaration.  Call sites are checked everywhere,
   because a call that cannot work must be refused wherever it is written.

3. **Float by value — SETTLED by Q2's answer: stays refused on both backends.**  `--native`
   emits typed `extern "C"` and could pass a double in an SSE register at no cost, but
   relaxing it there alone would reintroduce exactly the split arc C just closed, for a need
   arc B's pointer idiom already covers.  Unlike arity, there is no cheap "raise the ladder"
   move here — the interpreter's trampolines are integer-class by construction, so a uniform
   relaxation would mean a second, SSE-aware ladder.  Not worth it while pointers work; if it
   is ever wanted, it moves for both backends at once.
4. **Raising the interpreter ceiling — DONE in arc C: 12 → 32, and it is now the ceiling for
   both backends rather than the interpreter's alone.**  The ladder is one
   `extern "C" fn(u64 × N) -> u64` per rung and the rungs above 7 are all the same
   stack-passing shape, so extending it was mechanical as predicted.  "Unbounded" is still
   true and is why 32 is a stopping point with a reason (`dgemm_` at 26 slots) rather than a
   number picked for roundness; past it, arc D's shim generation is the answer.
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

Every cell above is now a guarantee probe in
`native::numeric_array_shapes_cross_identically_on_both_backends`, against values
`lc_selftest.c` computes in C.  The E2 write-back cell especially: the whole plan rests on
it, and nothing pinned it before.

**The lesson arc D cost, worth more than the arc.**  The fixture was written to loft's own
pointer-and-count shape, so the composition matrix could report "14 argument slots ✅" while
no real BLAS routine was bindable at all.  Every axis in the matrix was varied *inside* a
convention that was itself the thing under test — the axis held fixed for free.  What broke
it open was writing ONE C function to a foreign convention and pointing loft at it.

So: **when a matrix measures a boundary, at least one cell must be written by the other
side of it.** The fixture now has three (`lc_daxpby_`, `lc_dgemm_`, `lc_split_`), and its
README says why they are different.

## Cross-arc dependencies

- **@PLN24** — the `#c` binding machinery this plan builds on; the arity ladder and the
  boundary refusals are its.
- **@PLN102** — the compatibility doctrine that Q2 has to answer to.

## See also

- [PACKAGES.md § Direct C binding — `#c`](../PACKAGES.md) — the binding contract; arc A's
  home, and where the bare-pointer idiom is written for an author.
- [`src/c_signature.rs`](../../../src/c_signature.rs) — `plan` (the slot assignment, arc D),
  `boundary_refusals` (the float refusal, arc B), `MAX_C_ARITY` (arc C).
- [`src/c_call.rs`](../../../src/c_call.rs) — the `0..=32` trampoline ladder and the
  interpreter's marshalling.
- [`src/generation/mod.rs`](../../../src/generation/mod.rs) — `output_c_direct_call`, the
  `--native` half of the same mapping.
- [`tests/fixtures/c_abi/`](../../../tests/fixtures/c_abi/) — the `#c` fixture and its
  `cc`-only build.  Its README § *The FOREIGN half* is where arc D's lesson lives.
- [DESIGN_DECISIONS.md](../DESIGN_DECISIONS.md) § C106 (one arity ceiling), § C107 (the
  signature decides the count).
