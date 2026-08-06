// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN129 arc B — the database source, from the C surface up.
//
// The first test here is step 0's probe, GRADUATED rather than deleted: it is
// the question every later step rests on — can core drive a C library through
// `c_call::resolve` from Rust, with no rustc, no loft frame and no re-entrancy?
// Kept because the answer can regress: a change to symbol resolution would
// otherwise surface as a lazy fetch that mysteriously finds nothing.

#![cfg(all(feature = "native-extensions", unix))]

use std::ffi::{CStr, CString, c_char, c_int, c_void};

fn declare_sqlite() {
    loft::c_call::set_declared_libraries(vec![loft::data::CLibrary {
        name: "libsqlite3.so.0".to_string(),
        pkg_dir: String::new(),
        optional: true,
    }]);
}

#[test]
fn probe_core_can_drive_sqlite_through_resolve() {
    declare_sqlite();
    let Some(open) = loft::c_call::resolve("sqlite3_open") else {
        eprintln!("SKIP: libsqlite3.so.0 not installed");
        return;
    };
    let exec = loft::c_call::resolve("sqlite3_exec").expect("sqlite3_exec");
    let prepare = loft::c_call::resolve("sqlite3_prepare_v2").expect("sqlite3_prepare_v2");
    let step = loft::c_call::resolve("sqlite3_step").expect("sqlite3_step");
    let col_text = loft::c_call::resolve("sqlite3_column_text").expect("sqlite3_column_text");
    let finalize = loft::c_call::resolve("sqlite3_finalize").expect("sqlite3_finalize");
    let close = loft::c_call::resolve("sqlite3_close").expect("sqlite3_close");

    // Typed, not the u64 trampoline ladder: core knows the signature at compile
    // time, so it can have the ABI by construction the way `--native` does.
    type Open = extern "C" fn(*const c_char, *mut *mut c_void) -> c_int;
    type Exec = extern "C" fn(
        *mut c_void,
        *const c_char,
        *const c_void,
        *const c_void,
        *mut *mut c_char,
    ) -> c_int;
    type Prepare = extern "C" fn(
        *mut c_void,
        *const c_char,
        c_int,
        *mut *mut c_void,
        *mut *const c_char,
    ) -> c_int;
    type Step = extern "C" fn(*mut c_void) -> c_int;
    type ColText = extern "C" fn(*mut c_void, c_int) -> *const c_char;
    type Fin = extern "C" fn(*mut c_void) -> c_int;
    type Close = extern "C" fn(*mut c_void) -> c_int;

    let open: Open = unsafe { std::mem::transmute(open) };
    let exec: Exec = unsafe { std::mem::transmute(exec) };
    let prepare: Prepare = unsafe { std::mem::transmute(prepare) };
    let step: Step = unsafe { std::mem::transmute(step) };
    let col_text: ColText = unsafe { std::mem::transmute(col_text) };
    let finalize: Fin = unsafe { std::mem::transmute(finalize) };
    let close: Close = unsafe { std::mem::transmute(close) };

    let path = CString::new(":memory:").unwrap();
    let mut db: *mut c_void = std::ptr::null_mut();
    assert_eq!(open(path.as_ptr(), &raw mut db), 0, "sqlite3_open");
    assert!(!db.is_null());

    let ddl = CString::new(
        "CREATE TABLE person(id INTEGER PRIMARY KEY, name TEXT); \
         INSERT INTO person VALUES (42,'grace')",
    )
    .unwrap();
    assert_eq!(
        exec(
            db,
            ddl.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null_mut()
        ),
        0,
        "sqlite3_exec"
    );

    // The shape a fault needs: one derived SELECT, one row, one column value.
    let sql = CString::new("SELECT name FROM person WHERE id = 42").unwrap();
    let mut st: *mut c_void = std::ptr::null_mut();
    assert_eq!(
        prepare(db, sql.as_ptr(), -1, &raw mut st, std::ptr::null_mut()),
        0,
        "sqlite3_prepare_v2"
    );
    assert_eq!(step(st), 100, "SQLITE_ROW");
    let name = col_text(st, 0);
    assert!(!name.is_null());
    let name = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .to_string();
    assert_eq!(name, "grace");
    finalize(st);

    // A SECOND resolve in the same process must reuse the loaded handle rather
    // than re-open: `resolve` searches what is already loaded first.
    let again = loft::c_call::resolve("sqlite3_open").expect("second resolve");
    assert_eq!(again, open as *const ());

    close(db);
}

// ── Step 4: the source that executes ──────────────────────────────────────────

use loft::database::sql_source::{Cell, Driver, SqlConn, driver_of};

/// A scratch database seeded through sqlite's own CLI-free path: the connection
/// core opens is READ-ONLY, so the fixture is written with the raw symbols the
/// probe above already proved work.
fn seed(path: &std::path::Path, ddl: &str) {
    let exec = loft::c_call::resolve("sqlite3_exec").expect("sqlite3_exec");
    let open = loft::c_call::resolve("sqlite3_open").expect("sqlite3_open");
    let close = loft::c_call::resolve("sqlite3_close").expect("sqlite3_close");
    type Open = extern "C" fn(*const c_char, *mut *mut c_void) -> c_int;
    type Exec = extern "C" fn(
        *mut c_void,
        *const c_char,
        *const c_void,
        *const c_void,
        *mut *mut c_char,
    ) -> c_int;
    type Close = extern "C" fn(*mut c_void) -> c_int;
    let open: Open = unsafe { std::mem::transmute(open) };
    let exec: Exec = unsafe { std::mem::transmute(exec) };
    let close: Close = unsafe { std::mem::transmute(close) };

    let p = CString::new(path.to_string_lossy().as_ref()).unwrap();
    let mut db: *mut c_void = std::ptr::null_mut();
    assert_eq!(open(p.as_ptr(), &raw mut db), 0);
    let sql = CString::new(ddl).unwrap();
    assert_eq!(
        exec(
            db,
            sql.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null_mut()
        ),
        0,
        "seed failed"
    );
    close(db);
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("loft_pln129_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir.join("people.db")
}

#[test]
fn a_source_string_names_its_driver() {
    assert_eq!(
        driver_of("sqlite:people.db"),
        Some((Driver::Sqlite, "people.db"))
    );
    // An image path is not a database, and must stay the file source's business.
    assert_eq!(driver_of("people.store"), None);
    assert_eq!(driver_of("https://example.com/people.store"), None);
}

#[test]
fn the_derived_query_runs_and_its_columns_come_back() {
    declare_sqlite();
    if loft::c_call::resolve("sqlite3_open_v2").is_none() {
        eprintln!("SKIP: libsqlite3.so.0 not installed");
        return;
    }
    let path = scratch("exec");
    seed(
        &path,
        "CREATE TABLE person(id INTEGER PRIMARY KEY, name TEXT); \
         INSERT INTO person VALUES (1,'ada'),(42,'grace'),(7,NULL),(9,'')",
    );

    let conn = SqlConn::open(Driver::Sqlite, &path.to_string_lossy()).expect("open");
    // Exactly what `derive_select` emits for `hash<Person[id]>`.
    let rows = conn
        .query(
            "SELECT \"id\", \"name\" FROM \"person\" WHERE \"id\" = ?",
            &[Cell::Int(42)],
        )
        .expect("query");
    assert_eq!(rows.len(), 1, "one key, one row");
    assert_eq!(rows[0][0], Some(Cell::Int(42)));
    assert_eq!(rows[0][1], Some(Cell::Text("grace".to_string())));

    // The NULL crossing: SQL NULL, '' and a value are THREE answers, and a
    // binding that collapses any two of them is the bug @PLN23's fixture exists
    // to catch.
    let rows = conn
        .query(
            "SELECT \"name\" FROM \"person\" WHERE \"id\" = ?",
            &[Cell::Int(7)],
        )
        .expect("query");
    assert_eq!(rows[0][0], None, "SQL NULL is not the empty string");
    let rows = conn
        .query(
            "SELECT \"name\" FROM \"person\" WHERE \"id\" = ?",
            &[Cell::Int(9)],
        )
        .expect("query");
    assert_eq!(rows[0][0], Some(Cell::Text(String::new())));

    // A key that is not there is an ABSENCE — no rows, no error.
    let rows = conn
        .query(
            "SELECT \"id\", \"name\" FROM \"person\" WHERE \"id\" = ?",
            &[Cell::Int(999)],
        )
        .expect("query");
    assert!(rows.is_empty());

    // A text key binds as text, not as a number that happens to parse.
    let rows = conn
        .query(
            "SELECT \"id\" FROM \"person\" WHERE \"name\" = ?",
            &[Cell::Text("ada".to_string())],
        )
        .expect("query");
    assert_eq!(rows[0][0], Some(Cell::Int(1)));

    // A second open of the same target REUSES the handle, and this is what
    // PROVES it rather than merely restating it: with the file unlinked, a fresh
    // open could not possibly succeed, and the cached handle still reads. A
    // connection per fault would cost more than the eager load laziness
    // replaces.
    std::fs::remove_file(&path).expect("unlink");
    let again = SqlConn::open(Driver::Sqlite, &path.to_string_lossy()).expect("reopen is cached");
    let rows = again
        .query(
            "SELECT \"name\" FROM \"person\" WHERE \"id\" = ?",
            &[Cell::Int(42)],
        )
        .expect("the cached handle still reads");
    assert_eq!(rows[0][0], Some(Cell::Text("grace".to_string())));
}

#[test]
fn the_connection_is_read_only_and_a_bad_query_says_why() {
    declare_sqlite();
    if loft::c_call::resolve("sqlite3_open_v2").is_none() {
        eprintln!("SKIP: libsqlite3.so.0 not installed");
        return;
    }
    let path = scratch("readonly");
    seed(
        &path,
        "CREATE TABLE person(id INTEGER PRIMARY KEY, name TEXT); \
         INSERT INTO person VALUES (42,'grace')",
    );
    let conn = SqlConn::open(Driver::Sqlite, &path.to_string_lossy()).expect("open");

    // v1 is read-only, and the HANDLE enforces it — not a convention a later
    // caller can forget (README failure path 4).
    let err = conn
        .query("INSERT INTO person VALUES (1,'ada')", &[])
        .expect_err("a write must be refused");
    assert!(
        err.contains("readonly") || err.contains("read-only"),
        "{err}"
    );

    // A schema that drifted surfaces as the engine's own text rather than as an
    // absence — the distinction arc C is built on.
    //
    // And this cell is the reason the connection turns SQLite's double-quoted
    // string literals OFF. Measured with them on, against a table holding one
    // row: this query answered `[Some(Text("naam"))]` — no error, the COLUMN NAME
    // returned as data, for every row. The derived query quotes every identifier,
    // so a renamed column would have materialised its own name into the record
    // (failure path 9, in its cruellest form).
    let err = conn
        .query("SELECT \"naam\" FROM \"person\"", &[])
        .expect_err("a missing column must be an error, never a string literal");
    assert!(err.contains("naam"), "{err}");

    // An unreachable database is unreachable, not empty.
    let missing = SqlConn::open(Driver::Sqlite, "/nonexistent/dir/nope.db");
    assert!(missing.is_err());
}
