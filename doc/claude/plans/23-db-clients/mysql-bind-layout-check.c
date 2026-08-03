/* @PLN23 S4 — the check that keeps `maria/src/stmt.c`'s hand-declared MYSQL_BIND
 * honest.
 *
 * The shim cannot include <mysql.h>: a consumer machine has the runtime library,
 * not the -dev package, and the shim build passes no -I. So the struct is written
 * out there, and a wrong layout would be silent memory corruption rather than a
 * compile error. This is the ONLY place the two are compared, and it compiles
 * against the authoritative header.
 *
 * Re-run it whenever the MariaDB connector major changes:
 *
 *   apt-get download libmariadb-dev && dpkg-deb -x libmariadb-dev_*.deb hdr
 *   cc -I hdr/usr/include/mariadb -o check mysql-bind-layout-check.c && ./check
 *
 * Last run: 112 bytes, all 19 offsets equal — PASS.
 *
 * Keep the struct below in step with the one in maria/src/stmt.c; the point of
 * the check is that they are the same declaration.
 */
#include <mysql.h>
#include <stddef.h>
#include <stdio.h>

/* ---- the hand declaration, copied verbatim from the shim ---- */
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

#define CMP(f, g)                                                              \
  do {                                                                         \
    if (offsetof(LM_BIND, f) != offsetof(MYSQL_BIND, g)) {                     \
      printf("MISMATCH %-18s ours=%zu theirs=%zu\n", #f, offsetof(LM_BIND, f), \
             offsetof(MYSQL_BIND, g));                                         \
      bad++;                                                                   \
    } else {                                                                   \
      printf("ok       %-18s @%zu\n", #f, offsetof(LM_BIND, f));               \
    }                                                                          \
  } while (0)

int main(void) {
  int bad = 0;
  printf("sizeof ours=%zu theirs=%zu  align ours=%zu theirs=%zu\n",
         sizeof(LM_BIND), sizeof(MYSQL_BIND), _Alignof(LM_BIND),
         _Alignof(MYSQL_BIND));
  if (sizeof(LM_BIND) != sizeof(MYSQL_BIND))
    bad++;
  CMP(length, length);
  CMP(is_null, is_null);
  CMP(buffer, buffer);
  CMP(error, error);
  CMP(u_row_ptr, u);
  CMP(store_param_func, store_param_func);
  CMP(fetch_result, fetch_result);
  CMP(skip_result, skip_result);
  CMP(buffer_length, buffer_length);
  CMP(offset, offset);
  CMP(length_value, length_value);
  CMP(flags, flags);
  CMP(pack_length, pack_length);
  CMP(buffer_type, buffer_type);
  CMP(error_value, error_value);
  CMP(is_unsigned, is_unsigned);
  CMP(long_data_used, long_data_used);
  CMP(is_null_value, is_null_value);
  CMP(extension, extension);
  printf("MYSQL_TYPE_STRING=%d LONGLONG=%d NULL=%d DOUBLE=%d\n",
         (int)MYSQL_TYPE_STRING, (int)MYSQL_TYPE_LONGLONG, (int)MYSQL_TYPE_NULL,
         (int)MYSQL_TYPE_DOUBLE);
  printf("enum size=%zu\n", sizeof(enum enum_field_types));
  printf(bad ? "FAIL %d\n" : "PASS\n", bad);
  return bad != 0;
}
