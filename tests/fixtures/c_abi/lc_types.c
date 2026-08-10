/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * lc_types — implementation.  See lc_types.h for what each half is for, and
 * README.md for the matrix.  Plain C, no dependency beyond libc.
 */

#include "lc_types.h"

#include <stdarg.h>
#include <stdlib.h>
#include <string.h>

/* Built here rather than written as a literal: a `LL` suffix is not C89, and
 * the point of this fixture is that `cc` alone can build it anywhere. */
static int64_t lc_mask64(void) {
  return ((int64_t)LC_MASK64_HI << 32) | (int64_t)LC_MASK64_LO;
}

/* ---- integers ----------------------------------------------------------- */

int64_t lc_i64(int64_t v) { return v ^ lc_mask64(); }

int32_t lc_i32(int32_t v) { return v ^ (int32_t)LC_MASK64_HI; }

int16_t lc_i16(int16_t v) { return (int16_t)(v ^ (int16_t)0x5A5A); }

int8_t lc_i8(int8_t v) { return (int8_t)(v ^ (int8_t)0x5A); }

uint64_t lc_u64(uint64_t v) { return v ^ (uint64_t)lc_mask64(); }

uint32_t lc_u32(uint32_t v) { return v ^ (uint32_t)LC_MASK32; }

uint16_t lc_u16(uint16_t v) { return (uint16_t)(v ^ (uint16_t)0x5A5A); }

uint8_t lc_u8(uint8_t v) { return (uint8_t)(v ^ (uint8_t)0x5A); }

int32_t lc_neg_i32(int32_t v) { return -v; }

int64_t lc_raw_i64(int64_t v) { return v; }

/* ---- boolean, character ------------------------------------------------- */

int32_t lc_bool(int32_t b) { return b ? 0 : 1; }

int32_t lc_raw_bool(int32_t b) { return b; }

uint32_t lc_char(uint32_t codepoint) { return codepoint + 1; }

/* ---- text --------------------------------------------------------------- */

int64_t lc_strlen(const char *s) {
  if (s == 0) {
    return -1;
  }
  return (int64_t)strlen(s);
}

/* Length over an explicit `n` bytes, counting past interior NULs.  Paired
 * with `lc_strlen` this is the whole loft-text-vs-C-string question in two
 * calls: give both the same loft `text` holding an interior NUL and they
 * disagree exactly when the binding has silently truncated it. */
int64_t lc_len_upto(const char *s, int64_t n) {
  int64_t i;
  int64_t last;
  if (s == 0) {
    return -1;
  }
  last = 0;
  for (i = 0; i < n; i++) {
    if (s[i] != '\0') {
      last = i + 1;
    }
  }
  return last;
}

int32_t lc_byte_at(const char *s, int64_t i) {
  if (s == 0 || i < 0) {
    return -1;
  }
  return (int32_t)(unsigned char)s[i];
}

int32_t lc_is_null(const void *p) { return p == 0 ? 1 : 0; }

/* Borrowed: static storage, valid forever, must NOT be freed by the caller. */
const char *lc_static_text(void) { return "loft/c-abi"; }

/* Owned: the caller must hand it back to `lc_free`.  The pair exists because
 * "who frees a returned pointer" has no answer in the C type system, so a
 * binding has to carry the answer per function — this is the case that says
 * so out loud. */
char *lc_alloc_text(const char *s) {
  size_t n;
  char *out;
  if (s == 0) {
    return 0;
  }
  n = strlen(s);
  out = (char *)malloc(n + 2);
  if (out == 0) {
    return 0;
  }
  memcpy(out, s, n);
  out[n] = '!';
  out[n + 1] = '\0';
  return out;
}

void lc_free(void *p) { free(p); }

/* NULL is a routine answer in C, not a fault — `PQerrorMessage` returns it
 * whenever there is no error.  Returning it conditionally (rather than always)
 * keeps ONE binding covering both halves, so the two cells cannot drift apart. */
const char *lc_maybe_text(int64_t present) {
  return present != 0 ? "here" : 0;
}

/* 0xE9 is `e`-acute in Latin-1 and an invalid UTF-8 lead byte.  A C library
 * that hands back locale-encoded bytes is ordinary, and loft text is UTF-8 by
 * definition, so the crossing has to answer for this — identically on both
 * backends, and without taking the program down (C80). */
const char *lc_latin1_text(void) { return "caf\xE9"; }

/* ---- vectors ------------------------------------------------------------ */

int64_t lc_i64_sum(const int64_t *p, int64_t n) {
  int64_t i;
  int64_t total = 0;
  if (p == 0) {
    return -1;
  }
  for (i = 0; i < n; i++) {
    total += p[i] * (i + 1);
  }
  return total;
}

/* Weighted like `lc_i64_sum`, so a vector delivered in the wrong ORDER is
 * visible.  Separate from the i64 version because an element-width mistake
 * (reading a vector<i32> with a 64-bit stride) reads half the elements at
 * double stride and would otherwise still return a plausible number. */
int64_t lc_i32_sum(const int32_t *p, int64_t n) {
  int64_t i;
  int64_t total = 0;
  if (p == 0) {
    return -1;
  }
  for (i = 0; i < n; i++) {
    total += (int64_t)p[i] * (i + 1);
  }
  return total;
}

int64_t lc_strv_total(const char *const *v, int64_t n) {
  int64_t i;
  int64_t total = 0;
  if (v == 0) {
    return -1;
  }
  for (i = 0; i < n; i++) {
    if (v[i] == 0) {
      return -1;
    }
    total += (int64_t)strlen(v[i]) * (i + 1);
  }
  return total;
}

/* ---- opaque handles ----------------------------------------------------- */

struct lc_handle {
  int64_t magic;
  int64_t value;
};

#define LC_HANDLE_MAGIC 0x10F7C0DE

void *lc_open(int64_t seed) {
  struct lc_handle *h = (struct lc_handle *)malloc(sizeof(struct lc_handle));
  if (h == 0) {
    return 0;
  }
  h->magic = LC_HANDLE_MAGIC;
  h->value = seed;
  return (void *)h;
}

/* Every accessor checks the magic, so a handle that arrived mangled — the
 * pointer truncated to 32 bits, or a loft `integer` null (i64::MIN) passed
 * where a handle was expected — answers -1 instead of dereferencing it.  A
 * fixture that segfaults on a bad binding tells you less than one that
 * reports. */
int64_t lc_read(void *h) {
  struct lc_handle *p = (struct lc_handle *)h;
  if (p == 0 || p->magic != LC_HANDLE_MAGIC) {
    return -1;
  }
  return p->value;
}

int64_t lc_bump(void *h, int64_t by) {
  struct lc_handle *p = (struct lc_handle *)h;
  if (p == 0 || p->magic != LC_HANDLE_MAGIC) {
    return -1;
  }
  p->value += by;
  return p->value;
}

void lc_close(void *h) {
  struct lc_handle *p = (struct lc_handle *)h;
  if (p != 0 && p->magic == LC_HANDLE_MAGIC) {
    p->magic = 0;
    free(p);
  }
}

/* ---- arity ladder ------------------------------------------------------- */

int64_t lc_arity0(void) { return 0x10F7; }

int64_t lc_arity1(int64_t a) { return a * 2; }

int64_t lc_arity6(int64_t a, int64_t b, int64_t c, int64_t d, int64_t e,
                  int64_t f) {
  return a * 2 + b * 3 + c * 5 + d * 7 + e * 11 + f * 13;
}

int64_t lc_arity7(int64_t a, int64_t b, int64_t c, int64_t d, int64_t e,
                  int64_t f, int64_t g) {
  return a * 2 + b * 3 + c * 5 + d * 7 + e * 11 + f * 13 + g * 17;
}

int64_t lc_arity12(int64_t a, int64_t b, int64_t c, int64_t d, int64_t e,
                   int64_t f, int64_t g, int64_t h, int64_t i, int64_t j,
                   int64_t k, int64_t l) {
  return a * 2 + b * 3 + c * 5 + d * 7 + e * 11 + f * 13 + g * 17 + h * 19 +
         i * 23 + j * 29 + k * 31 + l * 37;
}

/* ---- boundary ----------------------------------------------------------- */

double lc_f64(double v) { return v * 2.0 + 0.5; }

float lc_f32(float v) { return v * 2.0f + 0.5f; }

int64_t lc_pair_dot(struct lc_pair p) { return p.x * 3 + p.y * 5; }

int64_t lc_divmod(int64_t a, int64_t b, int64_t *rem) {
  if (b == 0) {
    if (rem != 0) {
      *rem = 0;
    }
    return 0;
  }
  if (rem != 0) {
    *rem = a % b;
  }
  return a / b;
}

int64_t lc_var_sum(int32_t n, ...) {
  va_list ap;
  int32_t i;
  int64_t total = 0;
  va_start(ap, n);
  for (i = 0; i < n; i++) {
    total += va_arg(ap, int64_t);
  }
  va_end(ap);
  return total;
}

/* ---- shims -------------------------------------------------------------- */

/* `memcpy` rather than a union or a pointer cast: type-punning through a cast
 * is undefined behaviour under strict aliasing, and every compiler in use
 * turns this pair into the same register move at -O2. */
int64_t lc_shim_f64(int64_t bits) {
  double in;
  double out;
  int64_t result;
  memcpy(&in, &bits, sizeof in);
  out = lc_f64(in);
  memcpy(&result, &out, sizeof result);
  return result;
}

int64_t lc_shim_f32(int64_t bits) {
  float in;
  float out;
  int32_t word;
  int32_t result;
  word = (int32_t)bits;
  memcpy(&in, &word, sizeof in);
  out = lc_f32(in);
  memcpy(&result, &out, sizeof result);
  return (int64_t)(uint32_t)result;
}

int64_t lc_shim_pair_dot(int64_t x, int64_t y) {
  struct lc_pair p;
  p.x = x;
  p.y = y;
  return lc_pair_dot(p);
}

/* An out-parameter becomes two calls, one per output.  Cheap here; a function
 * whose out-params are expensive to recompute wants a handle instead, which
 * is what `lc_open`/`lc_read` is the pattern for. */
int64_t lc_shim_div(int64_t a, int64_t b) {
  int64_t rem = 0;
  return lc_divmod(a, b, &rem);
}

int64_t lc_shim_mod(int64_t a, int64_t b) {
  int64_t rem = 0;
  lc_divmod(a, b, &rem);
  return rem;
}

int64_t lc_shim_sum3(int64_t a, int64_t b, int64_t c) {
  return lc_var_sum(3, a, b, c);
}

/* ---- NUMERIC (@PLN128) -------------------------------------------------- */

int64_t lc_dsum_scaled(const double *p, int64_t n) {
  double s = 0.0;
  int64_t i;
  for (i = 0; i < n; i++) s += p[i];
  return (int64_t)(s * 1000.0);
}

void lc_daxpy(double *y, int64_t ny, const double *x, int64_t nx,
              int64_t a_milli) {
  double a = (double)a_milli / 1000.0;
  int64_t n = ny < nx ? ny : nx;
  int64_t i;
  for (i = 0; i < n; i++) y[i] = y[i] + a * x[i];
}

int64_t lc_scalar_ref(const int64_t *p, int64_t n) {
  return (n == 1) ? *p * 7 : -1;
}

void lc_shim_scale(double *out, int64_t n_out, const double *v, int64_t n_v) {
  if (n_out >= 1 && n_v >= 1) out[0] = v[0] * 2.0;
}

/* ---- A RETAINING API (@PLN128 Q5) ---------------------------------------
 * FFTW's plan/execute split in miniature, and the shape zlib's `z_stream`,
 * `sqlite3_bind_text(SQLITE_STATIC)` and every "context object" share: C keeps
 * a buffer pointer across TWO calls the caller makes.
 *
 * Handing loft's own vector to `lc_plan` is a use-after-free — loft frees it at
 * its last loft-visible use, which is that call.  The cure needs no language
 * feature: let C own the buffer (`lc_buf_alloc`), hold it as an opaque handle,
 * and copy in and out.  Then nothing loft owns is retained, and the buffer's
 * lifetime is the handle's. */

void *lc_buf_alloc(int64_t nbytes) {
  return malloc((size_t)nbytes);
}

void lc_buf_free(void *p) {
  free(p);
}

void *lc_plan(void *buf, int64_t n) {
  struct lc_plan_s *p = (struct lc_plan_s *)malloc(sizeof(struct lc_plan_s));
  if (p == 0) {
    return 0;
  }
  p->buf = (double *)buf;
  p->n = n;
  return (void *)p;
}

/* Reads the retained buffer on a LATER call, which is the whole point.
 * Position-weighted, so a buffer that moved or was reused answers a different
 * number rather than the right one by luck. */
int64_t lc_run(const void *plan) {
  const struct lc_plan_s *p = (const struct lc_plan_s *)plan;
  double s = 0.0;
  int64_t i;
  if (p == 0 || p->buf == 0) {
    return -1;
  }
  for (i = 0; i < p->n; i++) {
    s += p->buf[i] * (double)(i + 1);
  }
  return (int64_t)(s * 1000.0);
}

void lc_plan_free(void *plan) {
  free(plan);
}

/* ---- ELEMENT WIDTHS (@PLN128 arc E) -------------------------------------
 * One reader per element width loft may hand over.  A vector reaches C as a
 * pointer into loft's OWN element bytes, so these are what says the two sides
 * agree about the stride: a reader striding differently from the writer reads
 * garbage, and the weighted sums below make that visible rather than plausible
 * (an unweighted sum survives a reversed or shifted array). */

int64_t lc_u16_dot(const uint16_t *p, int64_t n) {
  int64_t s = 0;
  int64_t i;
  for (i = 0; i < n; i++) s += (int64_t)p[i] * (i + 1);
  return s;
}

int64_t lc_u8_dot(const unsigned char *p, int64_t n) {
  int64_t s = 0;
  int64_t i;
  for (i = 0; i < n; i++) s += (int64_t)p[i] * (i + 1);
  return s;
}

int64_t lc_u32_dot(const uint32_t *p, int64_t n) {
  int64_t s = 0;
  int64_t i;
  for (i = 0; i < n; i++) s += (int64_t)p[i] * (i + 1);
  return s;
}

int64_t lc_char_dot(const uint32_t *p, int64_t n) {
  int64_t s = 0;
  int64_t i;
  for (i = 0; i < n; i++) s += (int64_t)p[i] * (i + 1);
  return s;
}

int64_t lc_bool_dot(const unsigned char *p, int64_t n) {
  int64_t s = 0;
  int64_t i;
  for (i = 0; i < n; i++) s += (p[i] ? 1 : 0) * (i + 1);
  return s;
}

int64_t lc_f32_dot_milli(const float *p, int64_t n) {
  double s = 0.0;
  int64_t i;
  for (i = 0; i < n; i++) s += (double)p[i] * (double)(i + 1);
  return (int64_t)(s * 1000.0);
}

/* ---- FORTRAN SHAPE (@PLN128 arc D) --------------------------------------
 * Every argument by reference, no counts anywhere.  See lc_types.h. */

/* @PLN128 arc E — the level-1 BLAS *function* shape: bare pointers in, and the
 * answer comes back BY VALUE in an SSE register.  `ddot_`, `dnrm2_` and
 * `dasum_` are all this, and until the caller grew a float-returning rung none
 * of them could be bound without an ANSI-C shim per routine. */
double lc_ddot_(const int64_t *n, const double *x, const double *y) {
  double s = 0.0;
  int64_t i;
  for (i = 0; i < *n; i++) s += x[i] * y[i];
  return s;
}

/* The `float` twin — `sdot_`.  A single is not a narrowed double: it comes back
 * as a single in the same register, and reading those bits as a double is a
 * denormal. */
float lc_sdot_(const int64_t *n, const float *x, const float *y) {
  float s = 0.0f;
  int64_t i;
  for (i = 0; i < *n; i++) s += x[i] * y[i];
  return s;
}

void lc_daxpby_(const int64_t *n, const double *alpha, const double *x,
                const double *beta, double *y) {
  int64_t i;
  for (i = 0; i < *n; i++) y[i] = (*alpha) * x[i] + (*beta) * y[i];
}

int64_t lc_split_(const double *v, int64_t sel, const double *w, int64_t nw) {
  double s = v[0] * 100.0 + (double)sel * 10.0;
  int64_t i;
  for (i = 0; i < nw; i++) s += w[i] * (double)(i + 1);
  return (int64_t)(s * 1000.0);
}

void lc_dgemm_(const char *transa, const char *transb, const int64_t *m,
               const int64_t *n, const int64_t *k, const double *alpha,
               const double *a, const int64_t *lda, const double *b,
               const int64_t *ldb, const double *beta, double *c,
               const int64_t *ldc) {
  int64_t i, j, p;
  /* Only 'N'/'N' is implemented.  Reading the two chars is deliberate: a
   * binding that delivers them in the wrong place answers -1 everywhere
   * instead of quietly computing the right product from the rest. */
  if (*transa != 'N' || *transb != 'N') {
    for (j = 0; j < *n; j++)
      for (i = 0; i < *m; i++) c[i + j * (*ldc)] = -1.0;
    return;
  }
  for (j = 0; j < *n; j++) {
    for (i = 0; i < *m; i++) {
      double acc = 0.0;
      for (p = 0; p < *k; p++)
        acc += a[i + p * (*lda)] * b[p + j * (*ldb)];
      c[i + j * (*ldc)] = (*alpha) * acc + (*beta) * c[i + j * (*ldc)];
    }
  }
}
