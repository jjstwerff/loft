/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * lc_types — one loft type per function, so a marshalling fault names its type.
 *
 * The fixture @PLN24 (`#c "<symbol>"`, direct C-ABI binding) is measured
 * against.  Everything here is plain C: no rustc, no libffi, no loft header.
 * See README.md for the matrix and the hand-computed expected values.
 *
 * Two halves, and the split IS the design being tested:
 *   - DIRECT   — every argument and the return are integer-class, so they fit
 *                the fixed per-arity trampolines (`fn(u64, ...) -> u64`).
 *   - BOUNDARY — a float/double argument, a struct by value, varargs, or an
 *                out-parameter.  A trampoline cannot express these; each one
 *                is paired with an `lc_shim_*` that can.  If a boundary case
 *                turns out to need no shim, the plan's scope is wrong.
 */

#ifndef LC_TYPES_H
#define LC_TYPES_H

#if defined(__STDC_VERSION__) && __STDC_VERSION__ >= 199901L
#include <stdint.h>
#else
/* Strict C89 has no <stdint.h>.  loft's widths are exact, so spell them for
 * the two ABIs loft builds on: LP64 (Linux, macOS) and LLP64 (Windows). */
typedef signed char int8_t;
typedef unsigned char uint8_t;
typedef short int16_t;
typedef unsigned short uint16_t;
typedef int int32_t;
typedef unsigned int uint32_t;
#if defined(_WIN32)
typedef __int64 int64_t;
typedef unsigned __int64 uint64_t;
#else
typedef long int64_t;
typedef unsigned long uint64_t;
#endif
#endif

#if defined(_WIN32)
#define LC_API __declspec(dllexport)
#else
#define LC_API
#endif

#if defined(__cplusplus)
extern "C" {
#endif

/* Masks chosen with bits set in BOTH halves of the word: a 32-bit truncation
 * of a 64-bit value cannot produce the right answer, so the return falsifies
 * a wrong width instead of merely looking plausible. */
#define LC_MASK64_HI 0x01234567
#define LC_MASK64_LO 0x89ABCDEF
#define LC_MASK32 0x5A5A5A5A

/* ---- DIRECT: integers, one per loft width -------------------------------
 * Each is self-inverse (x ^ m ^ m == x), so a probe can round-trip instead of
 * hard-coding one expected number.  The plan's load-bearing question is
 * exactly this axis: loft `integer` is i64, C is int/long/size_t. */
LC_API int64_t lc_i64(int64_t v);
LC_API int32_t lc_i32(int32_t v);
LC_API int16_t lc_i16(int16_t v);
LC_API int8_t lc_i8(int8_t v);
LC_API uint64_t lc_u64(uint64_t v);
LC_API uint32_t lc_u32(uint32_t v);
LC_API uint16_t lc_u16(uint16_t v);
LC_API uint8_t lc_u8(uint8_t v);

/* The ABI oracle (the `native_scalar_pkg` lesson): returns a NEGATIVE value.
 * A binding that declares the return 64-bit while C returns `int` leaves the
 * upper half undefined — -1 then arrives as 4294967295, which `>= 0` checks
 * silently accept.  A positive-only probe cannot see that. */
LC_API int32_t lc_neg_i32(int32_t v);

/* Reports its argument unchanged.  Not a transform — an INSTRUMENT: it is how
 * a probe sees what loft's `null` actually became on the C side (i64::MIN for
 * `integer`, 255 for `boolean`, NULL for `text`) rather than inferring it. */
LC_API int64_t lc_raw_i64(int64_t v);

/* ---- DIRECT: boolean and character --------------------------------------
 * loft `boolean` is THREE-state (false 0 / true 1 / null 255) and C has no
 * such type, so both the mapping and the null are open questions.  `lc_bool`
 * decides, `lc_raw_bool` observes. */
LC_API int32_t lc_bool(int32_t b);
LC_API int32_t lc_raw_bool(int32_t b);
LC_API uint32_t lc_char(uint32_t codepoint);

/* ---- DIRECT: text -------------------------------------------------------
 * loft `text` is UTF-8 + a LENGTH; C is a NUL-terminated pointer.  The two
 * disagree about a string containing an interior NUL, and about who owns a
 * returned buffer — both are exercised here rather than assumed away. */
LC_API int64_t lc_strlen(const char *s);
LC_API int64_t lc_len_upto(const char *s, int64_t n);
LC_API int32_t lc_byte_at(const char *s, int64_t i);
LC_API int32_t lc_is_null(const void *p);
LC_API const char *lc_static_text(void);
LC_API char *lc_alloc_text(const char *s);
LC_API void lc_free(void *p);

/* The two answers a `char *` return can carry that loft `text` has no C
 * counterpart for, so a binding has to decide them rather than inherit them:
 * a NULL pointer (C's "no string"; loft's null is a CONTENT sentinel), and
 * bytes that are not UTF-8 (loft text is UTF-8 by definition; C is bytes).
 * Both must read identically on the two backends or the crossing is not one
 * mapping but two. */
LC_API const char *lc_maybe_text(int64_t present);
LC_API const char *lc_latin1_text(void);

/* ---- DIRECT: vectors (pointer + count) ---------------------------------- */
LC_API int64_t lc_i64_sum(const int64_t *p, int64_t n);
LC_API int64_t lc_i32_sum(const int32_t *p, int64_t n);
LC_API int64_t lc_strv_total(const char *const *v, int64_t n);

/* ---- DIRECT: opaque handles --------------------------------------------
 * The real target shape: `PGconn *` / `MYSQL *` cross as a loft `integer`
 * holding the pointer value.  open/read/bump/close is the whole lifecycle a
 * database binding needs, and it is what proves a pointer survives the round
 * trip through an i64 slot. */
LC_API void *lc_open(int64_t seed);
LC_API int64_t lc_read(void *h);
LC_API int64_t lc_bump(void *h, int64_t by);
LC_API void lc_close(void *h);

/* ---- DIRECT: arity ladder ----------------------------------------------
 * Arity is the ONLY dimension the trampoline set is parameterised on, so it
 * gets its own cells — and not evenly spaced: on the SysV x86-64 ABI the
 * first SIX integer arguments go in registers and the seventh onward on the
 * stack, so 6 and 7 straddle the boundary a hand-written trampoline is most
 * likely to get wrong.  Each argument is weighted by a distinct prime, so any
 * two arguments swapped changes the answer — a positional fault is visible,
 * where a plain sum would hide it. */
LC_API int64_t lc_arity0(void);
LC_API int64_t lc_arity1(int64_t a);
LC_API int64_t lc_arity6(int64_t a, int64_t b, int64_t c, int64_t d, int64_t e,
                         int64_t f);
LC_API int64_t lc_arity7(int64_t a, int64_t b, int64_t c, int64_t d, int64_t e,
                         int64_t f, int64_t g);
LC_API int64_t lc_arity12(int64_t a, int64_t b, int64_t c, int64_t d, int64_t e,
                          int64_t f, int64_t g, int64_t h, int64_t i, int64_t j,
                          int64_t k, int64_t l);

/* ---- NUMERIC: the shapes BLAS/LAPACK/FFTW are made of (@PLN128) ----------
 * loft expands a `vector<T>` into pointer-then-count, so each of these takes
 * the count C would otherwise have no way to know.  The load-bearing cell is
 * `lc_daxpy`: every BLAS and LAPACK routine returns its result by WRITING
 * THROUGH a caller-supplied pointer, so if loft could not see those writes the
 * numeric stack would not be bindable at all.
 *
 * Values are scaled to integers on return because a `double` return would trip
 * the very refusal this fixture documents — the point is the array crossing,
 * not the return convention. */
LC_API int64_t lc_dsum_scaled(const double *p, int64_t n);
LC_API void lc_daxpy(double *y, int64_t ny, const double *x, int64_t nx,
                     int64_t a_milli);

/* A Fortran-style scalar-by-reference: loft has no address-of, so a scalar
 * reaches C as a 1-ELEMENT vector — and therefore costs TWO C slots, which is
 * why `dgemm_`'s 13 by-reference arguments need a collapsing shim. */
LC_API int64_t lc_scalar_ref(const int64_t *p, int64_t n);

/* The idiom the float refusal prescribes: a scalar double in and out, entirely
 * by pointer, so no float→bits conversion (which loft does not have) is
 * needed anywhere. */
LC_API void lc_shim_scale(double *out, int64_t n_out, const double *v,
                          int64_t n_v);

/* ---- FORTRAN SHAPE (@PLN128 arc D) -------------------------------------
 * Everything above takes loft's pointer-AND-count shape, because it was
 * written for loft.  BLAS and LAPACK were not: Fortran passes EVERY argument
 * by reference, so each one is a BARE pointer and the routine learns the
 * length from a separate `n` — which is itself a bare pointer.  A fixture
 * that only ever declares a count cannot tell whether a real numeric library
 * binds, so these two declare none.
 *
 * The trailing underscore is the name a Fortran compiler emits, kept so the
 * declaration reads exactly like the one an author would write against
 * `libblas`. */

/* y := alpha*x + beta*y — the daxpy/dscal shape, with both scalars by
 * reference.  Proves the write-back half survives with no count present. */
LC_API void lc_daxpby_(const int64_t *n, const double *alpha, const double *x,
                       const double *beta, double *y);

/* The `dgemm_` argument list at full width: THIRTEEN by-reference arguments,
 * which is the case @PLN128 is sized around.  Column-major, and only the
 * 'N'/'N' (no-transpose) case is implemented — the two `char *` arguments are
 * read, so a binding that misplaces them is caught rather than ignored. */
LC_API void lc_dgemm_(const char *transa, const char *transb, const int64_t *m,
                      const int64_t *n, const int64_t *k, const double *alpha,
                      const double *a, const int64_t *lda, const double *b,
                      const int64_t *ldb, const double *beta, double *c,
                      const int64_t *ldc);

/* A signature where counted and bare vectors are MIXED, arranged so that a
 * left-to-right walk which takes a count whenever an integer follows a pointer
 * gets it wrong: `sel` is an integer sitting where `v`'s count would go, and
 * the count that IS present belongs to `w`.  Position-weighted, so a binding
 * that assigns the count to the wrong vector answers a different number rather
 * than the right one by luck. */
LC_API int64_t lc_split_(const double *v, int64_t sel, const double *w,
                         int64_t nw);

/* ---- A RETAINING API (@PLN128 Q5) ---------------------------------------
 * C keeps a buffer pointer across two calls the caller makes — FFTW's
 * plan/execute split, zlib's `z_stream`, `sqlite3_bind_text(SQLITE_STATIC)`.
 * Bound with a LOFT vector this is a use-after-free; bound with a C-owned
 * buffer held as an opaque handle it is ordinary, which is what the fixture
 * demonstrates. */
struct lc_plan_s {
  double *buf;
  int64_t n;
};

LC_API void *lc_buf_alloc(int64_t nbytes);
LC_API void lc_buf_free(void *p);
LC_API void *lc_plan(void *buf, int64_t n);
LC_API int64_t lc_run(const void *plan);
LC_API void lc_plan_free(void *plan);

/* ---- ELEMENT WIDTHS (@PLN128 arc E) -------------------------------------
 * A vector reaches C as a pointer into loft's own element bytes, so the loft
 * element type and the C pointee are two spellings of one layout.  One reader
 * per width, each POSITION-WEIGHTED so a reader striding differently from the
 * writer answers a different number rather than the right one by luck. */
LC_API int64_t lc_u32_dot(const uint32_t *p, int64_t n);
LC_API int64_t lc_u16_dot(const uint16_t *p, int64_t n);
LC_API int64_t lc_u8_dot(const unsigned char *p, int64_t n);
LC_API int64_t lc_char_dot(const uint32_t *p, int64_t n);
LC_API int64_t lc_bool_dot(const unsigned char *p, int64_t n);
LC_API int64_t lc_f32_dot_milli(const float *p, int64_t n);

/* The level-1 BLAS *function* shape: bare pointers in, the answer back BY VALUE
 * in an SSE register.  `ddot_`, `dnrm2_` and `dasum_` are all this. */
LC_API double lc_ddot_(const int64_t *n, const double *x, const double *y);
LC_API float lc_sdot_(const int64_t *n, const float *x, const float *y);

/* ---- BOUNDARY: what a trampoline cannot call --------------------------- */

/* Floating point travels in SSE registers, not the integer registers a
 * `fn(u64, ...) -> u64` trampoline uses.  Not a width problem — a different
 * register file, which is why no amount of casting reaches it. */
LC_API double lc_f64(double v);
LC_API float lc_f32(float v);

struct lc_pair {
  int64_t x;
  int64_t y;
};
LC_API int64_t lc_pair_dot(struct lc_pair p);

LC_API int64_t lc_divmod(int64_t a, int64_t b, int64_t *rem);
LC_API int64_t lc_var_sum(int32_t n, ...);

/* ---- The shims: the same capability, integer-class ---------------------
 * Each is the ANSI-C escape hatch the plan leans on, at its real size.  If
 * these are not small, "keep loft-core minimal, complexity in the shim" is
 * not a trade anyone should take. */
LC_API int64_t lc_shim_f64(int64_t bits);
LC_API int64_t lc_shim_f32(int64_t bits);
LC_API int64_t lc_shim_pair_dot(int64_t x, int64_t y);
LC_API int64_t lc_shim_div(int64_t a, int64_t b);
LC_API int64_t lc_shim_mod(int64_t a, int64_t b);
LC_API int64_t lc_shim_sum3(int64_t a, int64_t b, int64_t c);

#if defined(__cplusplus)
}
#endif

#endif /* LC_TYPES_H */
