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
 * @PLN138 — the RESULT sets below are one per CURSOR, not one per process; the
 * parameter array stays single.  The reasoning is at the results section.
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
/*
 * @PLN138 — one bind SET per cursor, not one per process.
 *
 * Everything below used to be a single set of `g_r*` globals, and the file said
 * so: "the one-statement-per-process limit of the other shims applies here
 * too".  That was honest while a cursor was state ON the connection, because a
 * connection could only hold one.  A cursor is its own type now, and two of them
 * can be walked at once — so a shared set would let the second `db_rows` rebind
 * the buffers the first is still fetching into, and the first cursor would start
 * answering the second's rows.  A wrong VALUE, silently, which is the failure
 * class this whole fixture exists to keep out.
 *
 * So a set is opened per cursor and closed with it.  `lm_result_open` hands back
 * a HANDLE — an index into the arena, biased by one so that 0 stays "no set" —
 * and every accessor takes it.  Handles are reused after a close, which keeps a
 * long-lived connection's arena the size of its high-water mark rather than its
 * total.
 *
 * The PARAMETER array above stays single.  A parameter's life is bind → execute,
 * with nothing in between: `mysql_stmt_execute` reads the values, and
 * `mysql_stmt_store_result` then buffers every row client-side, so a cursor is
 * done with its parameters before the next statement is prepared.  A set per
 * cursor there would cost memory to protect a window that does not exist.
 */

#define LM_COL_INIT 256

typedef struct {
  int used;
  LM_BIND *rb;
  char **rbuf;         /* the buffer bound for each column */
  unsigned long *rcap; /* how much of it the library may write */
  unsigned long *rlen; /* the TRUE length, set even when truncated */
  char *rnull;
  char *rerr;
  char **rover;        /* per-column overflow buffer, grown on demand */
  unsigned long *rovcap;
  LM_BIND over;        /* the bind `mysql_stmt_fetch_column` reads */
  unsigned long over_len;
  char over_err;
  long n;
  long cap_n;
} LM_SET;

static LM_SET *g_sets;
static long g_nsets;

/* The set behind a handle, or NULL — every accessor starts here, so a stale or
 * fabricated handle is refused in ONE place rather than in nine. */
static LM_SET *set_of(int64_t h) {
  if (h < 1 || h > g_nsets) {
    return 0;
  }
  if (!g_sets[h - 1].used) {
    return 0;
  }
  return &g_sets[h - 1];
}

/* Open a result set for `n` columns and bind each to a buffer of its own.
 * Returns a handle, or 0 when the allocation fails — which the caller reports
 * rather than proceeding with a short array the library would read past.
 *
 * Every column is bound as STRING, whatever the server's type — the cursor
 * contract is `col -> text?`, so the conversion belongs to the library rather
 * than to a second type map here. */
int64_t lm_result_open(int64_t n);
int64_t lm_result_open(int64_t n) {
  long i;
  long slot = -1;
  LM_SET *s;
  if (n < 0) {
    return 0;
  }
  for (i = 0; i < g_nsets; i++) {
    if (!g_sets[i].used) {
      slot = i;
      break;
    }
  }
  if (slot < 0) {
    LM_SET *grown =
        (LM_SET *)realloc(g_sets, (size_t)(g_nsets + 1) * sizeof(LM_SET));
    if (grown == 0) {
      return 0;
    }
    g_sets = grown;
    slot = g_nsets;
    memset(&g_sets[slot], 0, sizeof(LM_SET));
    g_nsets++;
  }
  s = &g_sets[slot];
  if (n > s->cap_n) {
    LM_BIND *b = (LM_BIND *)realloc(s->rb, (size_t)n * sizeof(LM_BIND));
    char **buf = (char **)realloc(s->rbuf, (size_t)n * sizeof(char *));
    unsigned long *cap =
        (unsigned long *)realloc(s->rcap, (size_t)n * sizeof(unsigned long));
    unsigned long *len =
        (unsigned long *)realloc(s->rlen, (size_t)n * sizeof(unsigned long));
    char *nu = (char *)realloc(s->rnull, (size_t)n);
    char *er = (char *)realloc(s->rerr, (size_t)n);
    char **ov = (char **)realloc(s->rover, (size_t)n * sizeof(char *));
    unsigned long *ovc =
        (unsigned long *)realloc(s->rovcap, (size_t)n * sizeof(unsigned long));
    if (b == 0 || buf == 0 || cap == 0 || len == 0 || nu == 0 || er == 0 ||
        ov == 0 || ovc == 0) {
      /* Keep whichever grew; the next open retries.  Nothing is bound yet, so a
       * partial grow is inert rather than inconsistent. */
      if (b) { s->rb = b; }
      if (buf) { s->rbuf = buf; }
      if (cap) { s->rcap = cap; }
      if (len) { s->rlen = len; }
      if (nu) { s->rnull = nu; }
      if (er) { s->rerr = er; }
      if (ov) { s->rover = ov; }
      if (ovc) { s->rovcap = ovc; }
      return 0;
    }
    s->rb = b;
    s->rbuf = buf;
    s->rcap = cap;
    s->rlen = len;
    s->rnull = nu;
    s->rerr = er;
    s->rover = ov;
    s->rovcap = ovc;
    for (i = s->cap_n; i < n; i++) {
      s->rbuf[i] = 0;
      s->rcap[i] = 0;
      s->rover[i] = 0;
      s->rovcap[i] = 0;
    }
    s->cap_n = n;
  }
  if (n > 0) {
    memset(s->rb, 0, (size_t)n * sizeof(LM_BIND));
  }
  for (i = 0; i < n; i++) {
    if (s->rbuf[i] == 0) {
      s->rbuf[i] = (char *)malloc(LM_COL_INIT + 1);
      if (s->rbuf[i] == 0) {
        return 0;
      }
      s->rcap[i] = LM_COL_INIT;
    }
    s->rlen[i] = 0;
    s->rnull[i] = 0;
    s->rerr[i] = 0;
    s->rb[i].buffer_type = LM_TYPE_STRING;
    s->rb[i].buffer = s->rbuf[i];
    s->rb[i].buffer_length = s->rcap[i];
    s->rb[i].length = &s->rlen[i];
    s->rb[i].is_null = &s->rnull[i];
    s->rb[i].error = &s->rerr[i];
  }
  s->n = n;
  s->used = 1;
  return slot + 1;
}

/* Return the set to the arena.  The BUFFERS are kept: a connection tends to run
 * result sets of similar shape, so keeping them makes a reused handle free,
 * and the arena is bounded by the high-water mark of live cursors either way.
 * Idempotent, because a cursor closes on exhaustion AND at scope end. */
void lm_result_close(int64_t h);
void lm_result_close(int64_t h) {
  LM_SET *s = set_of(h);
  if (s == 0) {
    return;
  }
  s->used = 0;
  s->n = 0;
}

void *lm_result_binds(int64_t h);
void *lm_result_binds(int64_t h) {
  LM_SET *s = set_of(h);
  if (s == 0 || s->n <= 0) {
    return 0;
  }
  return (void *)s->rb;
}

/* An unknown handle answers NULL rather than a value.  A caller that lost its
 * set has no row to read, and inventing one is the wrong half of the `text?`
 * contract. */
int lm_result_is_null(int64_t h, int64_t i);
int lm_result_is_null(int64_t h, int64_t i) {
  LM_SET *s = set_of(h);
  if (s == 0 || i < 0 || i >= s->n) {
    return 1;
  }
  return s->rnull[i] != 0;
}

/* The TRUE length of the column's value.  The library sets it even when the
 * value did not fit, which is what makes truncation detectable without relying
 * on the fetch return code. */
int64_t lm_result_len(int64_t h, int64_t i);
int64_t lm_result_len(int64_t h, int64_t i) {
  LM_SET *s = set_of(h);
  if (s == 0 || i < 0 || i >= s->n) {
    return 0;
  }
  return (long)s->rlen[i];
}

int64_t lm_result_cap(int64_t h, int64_t i);
int64_t lm_result_cap(int64_t h, int64_t i) {
  LM_SET *s = set_of(h);
  if (s == 0 || i < 0 || i >= s->n) {
    return 0;
  }
  return (long)s->rcap[i];
}

/* Prepare a bind for re-fetching one column that did not fit, and return its
 * address for `mysql_stmt_fetch_column`.
 *
 * It writes into a SEPARATE overflow buffer rather than growing the bound one.
 * `mysql_stmt_bind_result` copies the array into the statement, so the library
 * still holds the original pointer: reallocating that buffer would leave the
 * statement pointing at freed memory for the next row — a use-after-free that
 * only a long value would ever reach. */
void *lm_result_grow(int64_t h, int64_t i, int64_t need);
void *lm_result_grow(int64_t h, int64_t i, int64_t need) {
  LM_SET *s = set_of(h);
  if (s == 0 || i < 0 || i >= s->n || need < 0) {
    return 0;
  }
  if ((unsigned long)need > s->rovcap[i] || s->rover[i] == 0) {
    char *grown = (char *)realloc(s->rover[i], (size_t)need + 1);
    if (grown == 0) {
      return 0;
    }
    s->rover[i] = grown;
    s->rovcap[i] = (unsigned long)need;
  }
  memset(&s->over, 0, sizeof(s->over));
  s->over_len = 0;
  s->over_err = 0;
  s->over.buffer_type = LM_TYPE_STRING;
  s->over.buffer = s->rover[i];
  s->over.buffer_length = (unsigned long)need;
  s->over.length = &s->over_len;
  s->over.is_null = &s->rnull[i];
  s->over.error = &s->over_err;
  return (void *)&s->over;
}

/* The column's bytes, NUL-terminated.
 *
 * Which buffer holds them is DERIVED from the same fact the caller used to
 * decide whether to re-fetch — a value longer than the bound buffer is in the
 * overflow one — so there is no separate flag to keep in step. */
const char *lm_result_text(int64_t h, int64_t i);
const char *lm_result_text(int64_t h, int64_t i) {
  unsigned long n;
  LM_SET *s = set_of(h);
  if (s == 0 || i < 0 || i >= s->n) {
    return 0;
  }
  n = s->rlen[i];
  if (n > s->rcap[i]) {
    if (s->rover[i] == 0) {
      return 0;
    }
    if (n > s->rovcap[i]) {
      n = s->rovcap[i];
    }
    s->rover[i][n] = 0;
    return s->rover[i];
  }
  s->rbuf[i][n] = 0;
  return s->rbuf[i];
}
