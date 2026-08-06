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
