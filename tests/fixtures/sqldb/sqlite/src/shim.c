/* @PLN23 — sqlite hands its handles back through OUT-PARAMETERS
 * (`sqlite3_open(path, sqlite3 **ppDb)`), and loft has no way to pass the
 * address of a pointer it owns. The shim owns the slots instead.
 *
 * Deliberately free of any sqlite3 symbol: it is a place to put a pointer, not
 * a wrapper around the library, so it links against nothing and cannot drift
 * from whichever libsqlite3 the system has.
 *
 * The slots are static, so this is single-connection. That is a real limit and
 * it is here rather than hidden: a per-connection slot needs an allocator, and
 * this slice is proving the interface, not the pooling. */
static void *g_slot_a;
static void *g_slot_b;

void **ls_slot_a(void); void **ls_slot_a(void) { return &g_slot_a; }
void **ls_slot_b(void); void **ls_slot_b(void) { return &g_slot_b; }
void *ls_take_a(void);  void *ls_take_a(void)  { return g_slot_a; }
void *ls_take_b(void);  void *ls_take_b(void)  { return g_slot_b; }

/* SQLITE_TRANSIENT — the pointer `(void *)-1`, which tells sqlite to COPY the
 * bytes before returning.  That copy is what makes it correct to bind a loft
 * text: loft owns the buffer and makes no promise about it outliving the call,
 * so SQLITE_STATIC would leave sqlite holding a pointer into someone else's
 * memory.
 *
 * It is here because loft has no way to spell a negative pointer constant, not
 * because sqlite is involved — this function names a value and still references
 * no sqlite symbol, so the shim keeps compiling on a machine without the
 * library. */
void *ls_transient(void); void *ls_transient(void) { return (void *)-1; }
