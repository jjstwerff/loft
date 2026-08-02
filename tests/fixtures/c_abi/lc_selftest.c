/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * lc_selftest — the fixture judging itself, with no loft in the picture.
 *
 * `lc_types` is meant to be the ORACLE a `#c` binding is measured against, and
 * an oracle nobody validated is just a second opinion.  Every expected value
 * below is hand-computed and written out in the assertion, so when a loft
 * binding later disagrees with this library the disagreement is the binding's.
 *
 *   cc -o lc_selftest lc_selftest.c lc_types.c && ./lc_selftest
 *
 * Prints one line per failure and exits non-zero; silent and 0 when clean.
 */

#include "lc_types.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int failures = 0;

static void eq(const char *what, int64_t got, int64_t want) {
  if (got != want) {
    failures++;
    printf("FAIL %s: got %ld want %ld\n", what, (long)got, (long)want);
  }
}

int main(void) {
  int64_t mask;
  int64_t v64[4];
  int32_t v32[4];
  const char *strv[3];
  const char *embedded;
  void *h;
  char *owned;
  double f;
  float g;
  int64_t rem;

  mask = ((int64_t)LC_MASK64_HI << 32) | (int64_t)LC_MASK64_LO;

  /* Integers.  The round trip is the assertion: self-inverse means a probe
   * needs no magic constant, and the one-way check below pins the constant
   * anyway so a mask change cannot pass silently. */
  eq("i64 round trip", lc_i64(lc_i64(1234567890123)), 1234567890123);
  eq("i64 one way", lc_i64(0), mask);
  eq("i64 negative", lc_i64(lc_i64(-1)), -1);
  eq("i32 round trip", lc_i32(lc_i32(-77)), -77);
  eq("i16 round trip", lc_i16(lc_i16((int16_t)-300)), -300);
  eq("i8 round trip", lc_i8(lc_i8((int8_t)-5)), -5);
  eq("u32 one way", (int64_t)lc_u32(0), (int64_t)(uint32_t)LC_MASK32);
  eq("u8 round trip", (int64_t)lc_u8(lc_u8((uint8_t)200)), 200);

  /* The width oracle: negative, so a 32-bit return read as 64-bit is caught. */
  eq("neg i32", lc_neg_i32(1), -1);
  eq("raw i64 passes the sentinel through", lc_raw_i64(-42), -42);

  /* Boolean is three-state on the loft side; C sees whatever the binding
   * decided to send, which is why `lc_raw_bool` reports rather than judges. */
  eq("bool true", lc_bool(1), 0);
  eq("bool false", lc_bool(0), 1);
  eq("raw bool 255", lc_raw_bool(255), 255);
  eq("char", (int64_t)lc_char(97), 98);

  /* Text.  "loft" is 4 bytes; the interior-NUL case is the one that decides
   * whether loft `text` can cross as a C string at all. */
  eq("strlen", lc_strlen("loft"), 4);
  eq("strlen null", lc_strlen(0), -1);
  eq("byte_at", lc_byte_at("loft", 0), 108);
  eq("is_null yes", lc_is_null(0), 1);
  eq("is_null no", lc_is_null("x"), 0);
  embedded = "ab\0cd";
  eq("strlen stops at the interior NUL", lc_strlen(embedded), 2);
  eq("len_upto sees past it", lc_len_upto(embedded, 5), 5);
  eq("static text", lc_strlen(lc_static_text()), 10);
  owned = lc_alloc_text("abc");
  eq("alloc text", lc_strlen(owned), 4);
  eq("alloc text marks its copy", lc_byte_at(owned, 3), '!');
  lc_free(owned);

  /* Vectors: weighted, so order faults show. */
  v64[0] = 10;
  v64[1] = 20;
  v64[2] = 30;
  eq("i64 sum", lc_i64_sum(v64, 3), 10 * 1 + 20 * 2 + 30 * 3);
  v32[0] = 10;
  v32[1] = 20;
  v32[2] = 30;
  eq("i32 sum", lc_i32_sum(v32, 3), 140);
  eq("i64 sum null", lc_i64_sum(0, 3), -1);
  strv[0] = "a";
  strv[1] = "bb";
  strv[2] = "ccc";
  eq("strv total", lc_strv_total(strv, 3), 1 * 1 + 2 * 2 + 3 * 3);

  /* Handles: the database shape. */
  h = lc_open(1000);
  eq("handle read", lc_read(h), 1000);
  eq("handle bump", lc_bump(h, 7), 1007);
  lc_close(h);
  eq("handle rejects null", lc_read(0), -1);

  /* Arity, including the six-in-registers / seventh-on-the-stack boundary. */
  eq("arity0", lc_arity0(), 0x10F7);
  eq("arity1", lc_arity1(21), 42);
  eq("arity6", lc_arity6(1, 1, 1, 1, 1, 1), 2 + 3 + 5 + 7 + 11 + 13);
  eq("arity7", lc_arity7(1, 1, 1, 1, 1, 1, 1), 2 + 3 + 5 + 7 + 11 + 13 + 17);
  eq("arity7 weights the stack argument",
     lc_arity7(0, 0, 0, 0, 0, 0, 2) - lc_arity7(0, 0, 0, 0, 0, 0, 1), 17);
  eq("arity12", lc_arity12(1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1),
     2 + 3 + 5 + 7 + 11 + 13 + 17 + 19 + 23 + 29 + 31 + 37);

  /* Boundary + shim: each shim must answer exactly what the direct call does. */
  f = lc_f64(2.0);
  eq("f64 direct", f == 4.5 ? 1 : 0, 1);
  {
    double in = 2.0;
    int64_t in_bits;
    double out;
    int64_t out_bits;
    memcpy(&in_bits, &in, sizeof in_bits);
    out_bits = lc_shim_f64(in_bits);
    memcpy(&out, &out_bits, sizeof out);
    eq("f64 shim equals the direct call", out == f ? 1 : 0, 1);
  }
  g = lc_f32(2.0f);
  {
    float in = 2.0f;
    int32_t in_word;
    int32_t out_word;
    float out;
    memcpy(&in_word, &in, sizeof in_word);
    out_word = (int32_t)lc_shim_f32((int64_t)(uint32_t)in_word);
    memcpy(&out, &out_word, sizeof out);
    eq("f32 shim equals the direct call", out == g ? 1 : 0, 1);
  }
  {
    struct lc_pair p;
    p.x = 4;
    p.y = 5;
    eq("pair dot", lc_pair_dot(p), 4 * 3 + 5 * 5);
    eq("pair shim equals the direct call", lc_shim_pair_dot(4, 5),
       lc_pair_dot(p));
  }
  rem = 0;
  eq("divmod quotient", lc_divmod(17, 5, &rem), 3);
  eq("divmod remainder", rem, 2);
  eq("div shim", lc_shim_div(17, 5), 3);
  eq("mod shim", lc_shim_mod(17, 5), 2);
  eq("varargs", lc_var_sum(3, (int64_t)1, (int64_t)2, (int64_t)3), 6);
  eq("varargs shim", lc_shim_sum3(1, 2, 3), 6);

  if (failures != 0) {
    printf("%d failure(s)\n", failures);
    return 1;
  }
  return 0;
}
