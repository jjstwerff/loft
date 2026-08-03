/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * @PLN23 S4 — the array libpq reads bound parameters out of.
 *
 * `PQexecPrepared` takes `const char *const *paramValues`: an array of
 * pointers, any of which may be NULL for SQL NULL.  loft can hand C one string
 * at a time but has no way to build an array of them, so the array lives here.
 *
 * **Deliberately free of any libpq symbol.**  The shim owns MEMORY and never
 * calls the library — the same rule the sqlite and duckdb shims keep, and the
 * reason all three compile with `cc` on a machine that has never seen the
 * library they serve.
 *
 * The values are COPIED.  loft owns its text buffers and promises nothing about
 * them outliving the call, while libpq reads this array during
 * `PQexecPrepared` — so pointing the array at loft's bytes would be a bet on
 * two lifetimes nobody stated.  The copies are freed on the next reset, and the
 * final set is freed by the reset the next statement performs.
 *
 * A single static array, so this is one statement in flight per process — the
 * same single-slot limit the other shims carry, stated here rather than hidden.
 */
#include <stdlib.h>
#include <string.h>

static char **g_vals;
static int g_cap;
static int g_n;

static void release_values(void) {
  int i;
  for (i = 0; i < g_n; i++) {
    free(g_vals[i]);
    g_vals[i] = 0;
  }
}

/* Size the array for `n` parameters and clear every slot to NULL.
 * Returns 0 only when the allocation fails, which the caller reports rather
 * than proceeding with a short array. */
int lp_reset(int n);
int lp_reset(int n) {
  int i;
  release_values();
  if (n < 0) {
    return 0;
  }
  if (n > g_cap) {
    char **grown = (char **)realloc(g_vals, (size_t)n * sizeof(char *));
    if (grown == 0) {
      return 0;
    }
    g_vals = grown;
    g_cap = n;
  }
  for (i = 0; i < n; i++) {
    g_vals[i] = 0;
  }
  g_n = n;
  return 1;
}

int lp_set(int i, const char *s);
int lp_set(int i, const char *s) {
  size_t len;
  if (i < 0 || i >= g_n || s == 0) {
    return 0;
  }
  free(g_vals[i]);
  len = strlen(s);
  g_vals[i] = (char *)malloc(len + 1);
  if (g_vals[i] == 0) {
    return 0;
  }
  memcpy(g_vals[i], s, len + 1);
  return 1;
}

/* A NULL pointer in the array IS SQL NULL.  That is what keeps the empty
 * string — a real pointer to a zero byte — a different answer, which is most of
 * why a binding exists at all. */
int lp_set_null(int i);
int lp_set_null(int i) {
  if (i < 0 || i >= g_n) {
    return 0;
  }
  free(g_vals[i]);
  g_vals[i] = 0;
  return 1;
}

/* The address libpq reads, or NULL when the statement binds nothing —
 * `PQexecPrepared` accepts a NULL array only when nParams is 0. */
void *lp_vals(void);
void *lp_vals(void) { return g_n > 0 ? (void *)g_vals : (void *)0; }
