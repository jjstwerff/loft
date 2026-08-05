/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * @PLN23 S4 — the MYSQL_BIND arrays, which is where the ANSI-C shim earns its
 * keep (@PLN24 arc D).
 *
 * `mysql_stmt_bind_param` and `mysql_stmt_bind_result` each take an ARRAY OF
 * STRUCTS.  loft can pass a pointer and a scalar; it cannot lay out a struct,
 * let alone an array of them.  So the layout lives here.
 *
 * **The division of labour is the point: this file owns MEMORY, and loft makes
 * every library call.**  Not one mysql symbol is referenced below, so the shim
 * compiles with `cc` on a machine that has never had libmariadb — the same
 * property the sqlite and duckdb shims have, and the one that lets a `#c`
 * package be built anywhere.  A shim that called `mysql_stmt_bind_param` itself
 * would need the library at link time and would put that dependency back.
 *
 * ## The struct is hand-declared, and that is the risk this file carries
 *
 * There is no `mysql.h` here — a consumer machine has the runtime library, not
 * the -dev package, and the shim build passes no `-I`.  So `MYSQL_BIND` is
 * written out below rather than included, and a wrong layout would be silent
 * memory corruption rather than a compile error.
 *
 * It was therefore verified rather than recalled: compiled against the
 * authoritative `mariadb_stmt.h` of the matching release, comparing `sizeof`
 * and `offsetof` field by field — 112 bytes, all 19 offsets equal.  MariaDB's
 * layout is NOT MySQL's (MariaDB has `flags` where MySQL has `param_number`),
 * which is exactly why guessing was not an option; the `libmariadb.so.3`
 * soname in loft.toml is what pins the ABI this matches.
 *
 * The one-statement-per-process limit of the other shims applies here too.
 */
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

/* enum_field_types, the values used below. */
#define LM_TYPE_LONGLONG 8
#define LM_TYPE_NULL 6
#define LM_TYPE_STRING 254

/* MYSQL_BIND, MariaDB Connector/C 3.x.  `my_bool` is `char`; the three function
 * pointers are only ever zeroed, so their exact signatures do not matter — only
 * that they occupy one pointer each.  The union `u` is one pointer wide. */
typedef struct {
  unsigned long *length;
  char *is_null;
  void *buffer;
  char *error;
  void *u_row_ptr;
  void (*store_param_func)(void);
  void (*fetch_result)(void);
  void (*skip_result)(void);
  unsigned long buffer_length;
  unsigned long offset;
  unsigned long length_value;
  unsigned int flags;
  unsigned int pack_length;
  unsigned int buffer_type;
  char error_value;
  char is_unsigned;
  char long_data_used;
  char is_null_value;
  void *extension;
} LM_BIND;

/* ---- parameters ---------------------------------------------------------- */

static LM_BIND *g_pb;
static char **g_pcopy;         /* one owned copy per text parameter */
static unsigned long *g_plen;  /* the length each bind points at */
static char *g_pnull;          /* the null indicator each bind points at */
static long g_pn;
static long g_pcap;

static void free_param_copies(void) {
  long i;
  for (i = 0; i < g_pn; i++) {
    free(g_pcopy[i]);
    g_pcopy[i] = 0;
  }
}

/* Size the parameter array for `n` values and clear it.  Returns 0 on an
 * allocation failure, which the caller reports rather than proceeding with a
 * short array the library would read past. */
int lm_param_reset(int64_t n);
int lm_param_reset(int64_t n) {
  long i;
  if (n < 0) {
    return 0;
  }
  free_param_copies();
  if (n > g_pcap) {
    LM_BIND *b = (LM_BIND *)realloc(g_pb, (size_t)n * sizeof(LM_BIND));
    char **c = (char **)realloc(g_pcopy, (size_t)n * sizeof(char *));
    unsigned long *l =
        (unsigned long *)realloc(g_plen, (size_t)n * sizeof(unsigned long));
    char *nu = (char *)realloc(g_pnull, (size_t)n);
    if (b == 0 || c == 0 || l == 0 || nu == 0) {
      /* Keep whichever grew; the next reset retries.  Nothing is bound yet, so
       * a partial grow is inert rather than inconsistent. */
      if (b) { g_pb = b; }
      if (c) { g_pcopy = c; }
      if (l) { g_plen = l; }
      if (nu) { g_pnull = nu; }
      return 0;
    }
    g_pb = b;
    g_pcopy = c;
    g_plen = l;
    g_pnull = nu;
    g_pcap = n;
  }
  memset(g_pb, 0, (size_t)n * sizeof(LM_BIND));
  for (i = 0; i < n; i++) {
    g_pcopy[i] = 0;
    g_plen[i] = 0;
    g_pnull[i] = 0;
  }
  g_pn = n;
  return 1;
}

int lm_param_text(int64_t i, const char *s);
int lm_param_text(int64_t i, const char *s) {
  size_t len;
  if (i < 0 || i >= g_pn || s == 0) {
    return 0;
  }
  /* Copied, because the library reads the buffer during
   * `mysql_stmt_execute` and loft promises nothing about its text outliving
   * the call that handed it over. */
  free(g_pcopy[i]);
  len = strlen(s);
  g_pcopy[i] = (char *)malloc(len + 1);
  if (g_pcopy[i] == 0) {
    return 0;
  }
  memcpy(g_pcopy[i], s, len + 1);
  g_plen[i] = (unsigned long)len;
  g_pb[i].buffer_type = LM_TYPE_STRING;
  g_pb[i].buffer = g_pcopy[i];
  g_pb[i].buffer_length = (unsigned long)len;
  g_pb[i].length = &g_plen[i];
  g_pb[i].is_null = 0;
  return 1;
}

/* The integer is stored in the shim's own slot: the bind points at storage that
 * must survive until execute, and a loft local does not. */
static long long *g_pint;
static long g_pint_cap;

int lm_param_int(int64_t i, long long v);
int lm_param_int(int64_t i, long long v) {
  if (i < 0 || i >= g_pn) {
    return 0;
  }
  if (g_pn > g_pint_cap) {
    long long *grown =
        (long long *)realloc(g_pint, (size_t)g_pn * sizeof(long long));
    if (grown == 0) {
      return 0;
    }
    g_pint = grown;
    g_pint_cap = g_pn;
  }
  g_pint[i] = v;
  g_pb[i].buffer_type = LM_TYPE_LONGLONG;
  g_pb[i].buffer = &g_pint[i];
  g_pb[i].buffer_length = sizeof(long long);
  g_pb[i].length = 0;
  g_pb[i].is_null = 0;
  return 1;
}

/* SQL NULL: the type says NULL and the indicator is set.  Distinct from a
 * zero-length string, which is a real value with a real buffer. */
int lm_param_null(int64_t i);
int lm_param_null(int64_t i) {
  if (i < 0 || i >= g_pn) {
    return 0;
  }
  g_pnull[i] = 1;
  g_pb[i].buffer_type = LM_TYPE_NULL;
  g_pb[i].buffer = 0;
  g_pb[i].buffer_length = 0;
  g_pb[i].length = 0;
  g_pb[i].is_null = &g_pnull[i];
  return 1;
}

void *lm_param_binds(void);
void *lm_param_binds(void) { return g_pn > 0 ? (void *)g_pb : (void *)0; }

/* ---- results ------------------------------------------------------------- */

#define LM_COL_INIT 256

static LM_BIND *g_rb;
static char **g_rbuf;         /* the buffer bound for each column */
static unsigned long *g_rcap; /* how much of it the library may write */
static unsigned long *g_rlen; /* the TRUE length, set even when truncated */
static char *g_rnull;
static char *g_rerr;
static char **g_rover;         /* per-column overflow buffer, grown on demand */
static unsigned long *g_rovcap;
static LM_BIND g_over;         /* the single bind `mysql_stmt_fetch_column` reads */
static unsigned long g_over_len;
static char g_over_err;
static long g_rn;
static long g_rcap_n;

/* Size the result array for `n` columns and bind each to a buffer of its own.
 *
 * Every column is bound as STRING, whatever the server's type — the cursor
 * contract is `db_col -> text?`, so the conversion belongs to the library
 * rather than to a second type map here. */
int lm_result_reset(int64_t n);
int lm_result_reset(int64_t n) {
  long i;
  if (n < 0) {
    return 0;
  }
  if (n > g_rcap_n) {
    LM_BIND *b = (LM_BIND *)realloc(g_rb, (size_t)n * sizeof(LM_BIND));
    char **buf = (char **)realloc(g_rbuf, (size_t)n * sizeof(char *));
    unsigned long *cap =
        (unsigned long *)realloc(g_rcap, (size_t)n * sizeof(unsigned long));
    unsigned long *len =
        (unsigned long *)realloc(g_rlen, (size_t)n * sizeof(unsigned long));
    char *nu = (char *)realloc(g_rnull, (size_t)n);
    char *er = (char *)realloc(g_rerr, (size_t)n);
    char **ov = (char **)realloc(g_rover, (size_t)n * sizeof(char *));
    unsigned long *ovc =
        (unsigned long *)realloc(g_rovcap, (size_t)n * sizeof(unsigned long));
    if (b == 0 || buf == 0 || cap == 0 || len == 0 || nu == 0 || er == 0 ||
        ov == 0 || ovc == 0) {
      if (b) { g_rb = b; }
      if (buf) { g_rbuf = buf; }
      if (cap) { g_rcap = cap; }
      if (len) { g_rlen = len; }
      if (nu) { g_rnull = nu; }
      if (er) { g_rerr = er; }
      if (ov) { g_rover = ov; }
      if (ovc) { g_rovcap = ovc; }
      return 0;
    }
    g_rb = b;
    g_rbuf = buf;
    g_rcap = cap;
    g_rlen = len;
    g_rnull = nu;
    g_rerr = er;
    g_rover = ov;
    g_rovcap = ovc;
    for (i = g_rcap_n; i < n; i++) {
      g_rbuf[i] = 0;
      g_rcap[i] = 0;
      g_rover[i] = 0;
      g_rovcap[i] = 0;
    }
    g_rcap_n = n;
  }
  memset(g_rb, 0, (size_t)n * sizeof(LM_BIND));
  for (i = 0; i < n; i++) {
    if (g_rbuf[i] == 0) {
      g_rbuf[i] = (char *)malloc(LM_COL_INIT + 1);
      if (g_rbuf[i] == 0) {
        return 0;
      }
      g_rcap[i] = LM_COL_INIT;
    }
    g_rlen[i] = 0;
    g_rnull[i] = 0;
    g_rerr[i] = 0;
    g_rb[i].buffer_type = LM_TYPE_STRING;
    g_rb[i].buffer = g_rbuf[i];
    g_rb[i].buffer_length = g_rcap[i];
    g_rb[i].length = &g_rlen[i];
    g_rb[i].is_null = &g_rnull[i];
    g_rb[i].error = &g_rerr[i];
  }
  g_rn = n;
  return 1;
}

void *lm_result_binds(void);
void *lm_result_binds(void) { return g_rn > 0 ? (void *)g_rb : (void *)0; }

int lm_result_is_null(int64_t i);
int lm_result_is_null(int64_t i) {
  if (i < 0 || i >= g_rn) {
    return 1;
  }
  return g_rnull[i] != 0;
}

/* The TRUE length of the column's value.  The library sets it even when the
 * value did not fit, which is what makes truncation detectable without relying
 * on the fetch return code. */
int64_t lm_result_len(int64_t i);
int64_t lm_result_len(int64_t i) {
  if (i < 0 || i >= g_rn) {
    return 0;
  }
  return (long)g_rlen[i];
}

int64_t lm_result_cap(int64_t i);
int64_t lm_result_cap(int64_t i) {
  if (i < 0 || i >= g_rn) {
    return 0;
  }
  return (long)g_rcap[i];
}

/* Prepare a bind for re-fetching one column that did not fit, and return its
 * address for `mysql_stmt_fetch_column`.
 *
 * It writes into a SEPARATE overflow buffer rather than growing the bound one.
 * `mysql_stmt_bind_result` copies the array into the statement, so the library
 * still holds the original pointer: reallocating that buffer would leave the
 * statement pointing at freed memory for the next row — a use-after-free that
 * only a long value would ever reach. */
void *lm_result_grow(int64_t i, int64_t need);
void *lm_result_grow(int64_t i, int64_t need) {
  if (i < 0 || i >= g_rn || need < 0) {
    return 0;
  }
  if ((unsigned long)need > g_rovcap[i] || g_rover[i] == 0) {
    char *grown = (char *)realloc(g_rover[i], (size_t)need + 1);
    if (grown == 0) {
      return 0;
    }
    g_rover[i] = grown;
    g_rovcap[i] = (unsigned long)need;
  }
  memset(&g_over, 0, sizeof(g_over));
  g_over_len = 0;
  g_over_err = 0;
  g_over.buffer_type = LM_TYPE_STRING;
  g_over.buffer = g_rover[i];
  g_over.buffer_length = (unsigned long)need;
  g_over.length = &g_over_len;
  g_over.is_null = &g_rnull[i];
  g_over.error = &g_over_err;
  return (void *)&g_over;
}

/* The column's bytes, NUL-terminated.
 *
 * Which buffer holds them is DERIVED from the same fact the caller used to
 * decide whether to re-fetch — a value longer than the bound buffer is in the
 * overflow one — so there is no separate flag to keep in step. */
const char *lm_result_text(int64_t i);
const char *lm_result_text(int64_t i) {
  unsigned long n;
  if (i < 0 || i >= g_rn) {
    return 0;
  }
  n = g_rlen[i];
  if (n > g_rcap[i]) {
    if (g_rover[i] == 0) {
      return 0;
    }
    if (n > g_rovcap[i]) {
      n = g_rovcap[i];
    }
    g_rover[i][n] = 0;
    return g_rover[i];
  }
  g_rbuf[i][n] = 0;
  return g_rbuf[i];
}
