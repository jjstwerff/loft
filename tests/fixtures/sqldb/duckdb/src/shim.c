/* @PLN23 / @PLN24 arc G — the places duckdb writes into, owned by loft.
 *
 * duckdb hands its handles back through OUT-PARAMETERS
 * (`duckdb_open(path, duckdb_database *out)`), and it makes the CALLER allocate
 * the `duckdb_result` a query fills in. loft can pass neither the address of a
 * pointer it owns nor a struct by value, so the shim owns both — the same trade
 * the sqlite shim makes, for the same reason.
 *
 * **Deliberately free of any duckdb symbol.** That is not tidiness: this shim is
 * compiled by `cc` at parse time on EVERY machine, including the ones that have
 * no libduckdb at all, which is the whole point of declaring the library
 * optional. A shim that called into duckdb would need it at link time and would
 * put back exactly the hard dependency arc G removes. So it is a place to put
 * bytes, never a wrapper around the library, and it cannot drift from whichever
 * libduckdb the machine has.
 *
 * The database, connection and statement slots are static, so this is
 * single-connection — a real limit, stated here rather than hidden, and the same
 * one the sqlite shim carries. A per-connection slot needs an allocator; this
 * fixture proves the interface.
 *
 * @PLN138 — the RESULT is the exception, and it had to be. A cursor is its own
 * type now and two can be walked at once, so a single `duckdb_result` would let
 * the second `db_rows` overwrite the struct the first is still reading rows out
 * of: a wrong VALUE, silently. Results therefore come from an ARENA, one slot
 * per live cursor, handed out as a handle. The statement stays single because
 * its life is prepare → execute → destroy with nothing in between — duckdb
 * materialises the result, so the statement is gone before the cursor is read.
 */

/* The database and connection handles, each written by duckdb through its
 * address and read back afterwards. `duckdb_close` / `duckdb_disconnect` take
 * the same address, so one slot serves the whole life of a handle. */
static void *g_db;
static void *g_conn;

/* Storage for `duckdb_result`s — one per live cursor (@PLN138).
 *
 * Each slot is sized rather than declared, because declaring it needs
 * `duckdb.h` and this file must compile with duckdb absent. 256 bytes is safe
 * for a specific reason: duckdb's C API makes the CALLER allocate this struct,
 * so its size is part of the library's stable ABI — growing it would break every
 * existing caller — and it is six pointer-width fields (48 bytes on a 64-bit
 * host) today. `long long` gives the alignment any of those fields could need.
 */
#include <stdlib.h>
#include <string.h>

#define LD_RESULT_WORDS 32

typedef struct {
  int used;
  long long words[LD_RESULT_WORDS];
} LD_RESULT;

static LD_RESULT *g_results;
static long g_nresults;

/* Take a result slot, zeroed.  Returns a handle biased by one, so 0 stays "no
 * result"; 0 also means the allocation failed, which the caller reports rather
 * than handing duckdb an address it does not own. */
long long ld_result_open(void);
long long ld_result_open(void) {
  long i;
  for (i = 0; i < g_nresults; i++) {
    if (!g_results[i].used) {
      g_results[i].used = 1;
      memset(g_results[i].words, 0, sizeof(g_results[i].words));
      return i + 1;
    }
  }
  {
    LD_RESULT *grown = (LD_RESULT *)realloc(
        g_results, (size_t)(g_nresults + 1) * sizeof(LD_RESULT));
    if (grown == 0) {
      return 0;
    }
    g_results = grown;
    memset(&g_results[g_nresults], 0, sizeof(LD_RESULT));
    g_results[g_nresults].used = 1;
    g_nresults++;
    return g_nresults;
  }
}

/* The address duckdb writes into and reads back — every result call takes it.
 * An unknown handle answers NULL, which every duckdb entry point treats as a
 * refusal rather than a fault. */
void *ld_result_at(long long h);
void *ld_result_at(long long h) {
  if (h < 1 || h > g_nresults) {
    return 0;
  }
  if (!g_results[h - 1].used) {
    return 0;
  }
  return (void *)g_results[h - 1].words;
}

/* Return the slot for reuse.  loft calls `duckdb_destroy_result` on the address
 * FIRST — this only reclaims the storage, so it must never be the thing that
 * frees duckdb's own buffers.  Idempotent, because a cursor closes on exhaustion
 * and again at scope end. */
void ld_result_close(long long h);
void ld_result_close(long long h) {
  if (h < 1 || h > g_nresults) {
    return;
  }
  g_results[h - 1].used = 0;
}

void **ld_slot_db(void);
void **ld_slot_db(void) { return &g_db; }

void **ld_slot_conn(void);
void **ld_slot_conn(void) { return &g_conn; }

/* @PLN23 S4 — the prepared statement, which duckdb also hands back through an
 * out-parameter (`duckdb_prepare(conn, sql, duckdb_prepared_statement *out)`)
 * and takes the address of again to destroy.  One slot serves both, exactly as
 * the database and connection handles above do. */
static void *g_stmt;

void **ld_slot_stmt(void);
void **ld_slot_stmt(void) { return &g_stmt; }

void *ld_take_stmt(void);
void *ld_take_stmt(void) { return g_stmt; }

void *ld_take_db(void);
void *ld_take_db(void) { return g_db; }

void *ld_take_conn(void);
void *ld_take_conn(void) { return g_conn; }

/* Hand a `char *` back as something loft will read as text.
 *
 * `duckdb_value_varchar` returns memory the CALLER must release with
 * `duckdb_free`, and loft's `char *` → `text` crossing copies the bytes and
 * never frees the pointer (@PLN24 arc D). Binding that return as `text`
 * directly would therefore leak one string per cell read. So the loft side
 * takes the POINTER, converts it here, and frees it — this function is the
 * conversion, and it is the identity on the pointer because loft's own crossing
 * does the copying.
 */
const char *ld_str(void *p);
const char *ld_str(void *p) { return (const char *)p; }
