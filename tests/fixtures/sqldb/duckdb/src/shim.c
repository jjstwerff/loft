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
 * The slots are static, so this is single-connection — a real limit, stated
 * here rather than hidden, and the same one the sqlite shim carries. A
 * per-connection slot needs an allocator; this fixture proves the interface.
 */

/* The database and connection handles, each written by duckdb through its
 * address and read back afterwards. `duckdb_close` / `duckdb_disconnect` take
 * the same address, so one slot serves the whole life of a handle. */
static void *g_db;
static void *g_conn;

/* Storage for one `duckdb_result`.
 *
 * Sized rather than declared, because declaring it needs `duckdb.h` and this
 * file must compile with duckdb absent. 256 bytes is safe for a specific
 * reason: duckdb's C API makes the CALLER allocate this struct, so its size is
 * part of the library's stable ABI — growing it would break every existing
 * caller — and it is six pointer-width fields (48 bytes on a 64-bit host)
 * today. `long long` gives the alignment any of those fields could need.
 */
static long long g_result[32];

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

void *ld_result(void);
void *ld_result(void) { return g_result; }

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
