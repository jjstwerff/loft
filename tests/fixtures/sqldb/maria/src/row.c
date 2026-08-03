/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * @PLN23 S3 — the ANSI-C shim, at its real size.
 *
 * `mysql_fetch_row` hands back a `char **`: an array of column pointers, any of
 * which is NULL for SQL NULL.  loft carries the array as an opaque handle but
 * has no way to index it, so this is the whole shim — one indexing step, and the
 * NULL it may find passes straight through for loft to read as null.
 *
 * A defensive `row == 0` is not paranoia here: it is the no-server path the test
 * uses to exercise this shim and the `text?` crossing with no database at all. */
const char *lm_row_col(const char *const *row, long i);
const char *lm_row_col(const char *const *row, long i) {
  if (row == 0 || i < 0) { return 0; }
  return row[i];
}
