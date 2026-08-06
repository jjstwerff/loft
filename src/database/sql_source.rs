// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN129 arc B step 4 — the database source that EXECUTES.
//
// Core sends a derived string and reads back a row. That is the whole surface,
// and it is deliberately narrow: no SQL knowledge lives here beyond "run this
// and hand me the columns", because the query was already derived from the
// store's own schema (`sql_query.rs`) and the dialect facts were declared with
// the mapping.
//
// **Rust-side, called through `c_call::resolve`.** The interpreter cannot make a
// synchronous loft call from inside a lookup — `State::fn_call` redirects the
// instruction pointer rather than nesting an interpreter — so a source that was
// a loft function would need the bytecode to resume a lookup after a callback
// (QUERIES.md § arc B). Resolving the symbols from Rust needs none of that, and
// step 0 measured it: no rustc, no loft frame, no re-entrancy.
//
// **Read-only, and enforced by the connection rather than by convention**: the
// handle is opened `SQLITE_OPEN_READONLY`, so a write cannot reach the source
// even by mistake (README failure path 4).

#![cfg(feature = "native-extensions")]

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::sync::{Mutex, PoisonError};

/// One value crossing the boundary. `None` at the row level is SQL `NULL`, which
/// stays distinct from `Text("")` — the distinction @PLN23's uniform fixture
/// exists to preserve, and the one a binding is for.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Int(i64),
    Real(f64),
    Text(String),
}

/// One row, in the SELECT's column order. `None` is SQL `NULL`.
pub type Row = Vec<Option<Cell>>;

/// The engines core can drive. One entry per C API, because the entry points
/// really are different names — `sqlite3_open_v2` and `PQconnectdb` are not
/// interchangeable, and pretending otherwise would need a shim in the middle
/// that someone has to build and ship.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Driver {
    Sqlite,
}

/// SQLite's own constants, spelled where they are used rather than imported from
/// a crate — this file links nothing.
mod sq {
    pub const OPEN_READONLY: i32 = 0x0000_0001;
    pub const ROW: i32 = 100;
    pub const DONE: i32 = 101;
    pub const NULL_TYPE: i32 = 5;
    pub const INTEGER: i32 = 1;
    pub const FLOAT: i32 = 2;
    /// `SQLITE_DBCONFIG_DQS_DDL` / `_DML` — accept a double-quoted string as a
    /// string LITERAL when it resolves to no identifier.
    pub const DQS_DDL: i32 = 1013;
    pub const DQS_DML: i32 = 1014;
}

type OpenV2 = extern "C" fn(*const c_char, *mut *mut c_void, c_int, *const c_char) -> c_int;
type Prepare =
    extern "C" fn(*mut c_void, *const c_char, c_int, *mut *mut c_void, *mut *const c_char) -> c_int;
type Step = extern "C" fn(*mut c_void) -> c_int;
type ColCount = extern "C" fn(*mut c_void) -> c_int;
type ColType = extern "C" fn(*mut c_void, c_int) -> c_int;
type ColInt = extern "C" fn(*mut c_void, c_int) -> i64;
type ColDouble = extern "C" fn(*mut c_void, c_int) -> f64;
type ColText = extern "C" fn(*mut c_void, c_int) -> *const c_char;
type BindInt = extern "C" fn(*mut c_void, c_int, i64) -> c_int;
type BindText = extern "C" fn(*mut c_void, c_int, *const c_char, c_int, *const c_void) -> c_int;
type BindDouble = extern "C" fn(*mut c_void, c_int, f64) -> c_int;
type Finalize = extern "C" fn(*mut c_void) -> c_int;
type ErrMsg = extern "C" fn(*mut c_void) -> *const c_char;
/// `sqlite3_db_config` is variadic, and declared as such rather than as a fixed
/// arity: calling a variadic C function through a non-variadic pointer happens to
/// work on SysV x86-64 and is undefined everywhere the ABI differs.
type DbConfig = unsafe extern "C" fn(*mut c_void, c_int, ...) -> c_int;

/// Every symbol this driver needs, resolved once.
///
/// All of them or none: a partial resolve is exactly the case
/// `c_library_available` exists for — a name-alike library that loads, exports a
/// subset, and faults on the call (loft#770).
struct SqliteApi {
    open_v2: OpenV2,
    prepare: Prepare,
    step: Step,
    col_count: ColCount,
    col_type: ColType,
    col_int: ColInt,
    col_double: ColDouble,
    col_text: ColText,
    bind_int: BindInt,
    bind_text: BindText,
    bind_double: BindDouble,
    finalize: Finalize,
    errmsg: ErrMsg,
    close: Step,
    db_config: DbConfig,
}

impl SqliteApi {
    fn resolve() -> Result<SqliteApi, String> {
        macro_rules! sym {
            ($name:literal, $ty:ty) => {{
                let Some(p) = crate::c_call::resolve($name) else {
                    return Err(format!(
                        "sqlite is not available — `{}` did not resolve; declare it with \
                         `[c] optional-libs = \"libsqlite3.so.0\"`",
                        $name
                    ));
                };
                let f: $ty = unsafe { std::mem::transmute(p) };
                f
            }};
        }
        Ok(SqliteApi {
            open_v2: sym!("sqlite3_open_v2", OpenV2),
            prepare: sym!("sqlite3_prepare_v2", Prepare),
            step: sym!("sqlite3_step", Step),
            col_count: sym!("sqlite3_column_count", ColCount),
            col_type: sym!("sqlite3_column_type", ColType),
            col_int: sym!("sqlite3_column_int64", ColInt),
            col_double: sym!("sqlite3_column_double", ColDouble),
            col_text: sym!("sqlite3_column_text", ColText),
            bind_int: sym!("sqlite3_bind_int64", BindInt),
            bind_text: sym!("sqlite3_bind_text", BindText),
            bind_double: sym!("sqlite3_bind_double", BindDouble),
            finalize: sym!("sqlite3_finalize", Finalize),
            errmsg: sym!("sqlite3_errmsg", ErrMsg),
            close: sym!("sqlite3_close", Step),
            db_config: sym!("sqlite3_db_config", DbConfig),
        })
    }
}

/// Open connections, keyed by the target they were opened for.
///
/// A connection per FAULT would make laziness cost more than the eager load it
/// replaces, so the handle is opened once and reused — which is also what arc D
/// needs, since a database pins a snapshot by holding one connection rather than
/// by stat'ing a file.
///
/// The handle is kept as a `usize` because a raw pointer is neither `Send` nor
/// `Sync` and this table is process-wide. SQLite's default build is serialized
/// (`SQLITE_THREADSAFE=1`), so the handle itself tolerates the sharing; the
/// mutex here is what orders loft's own use of it.
static CONNECTIONS: Mutex<Option<HashMap<String, usize>>> = Mutex::new(None);

/// A live connection to one database, and the symbols to drive it.
pub struct SqlConn {
    api: SqliteApi,
    db: *mut c_void,
    target: String,
}

impl SqlConn {
    /// Open (or reuse) a connection to `target`, which is the part of a lazy
    /// source string after its `sqlite:` prefix.
    ///
    /// # Errors
    /// When the library is not available, or the database cannot be opened
    /// read-only — both of which are arc C's *unreachable*, never an absence.
    pub fn open(driver: Driver, target: &str) -> Result<SqlConn, String> {
        let Driver::Sqlite = driver;
        let api = SqliteApi::resolve()?;
        let mut guard = CONNECTIONS.lock().unwrap_or_else(PoisonError::into_inner);
        let table = guard.get_or_insert_with(HashMap::new);
        if let Some(&handle) = table.get(target) {
            return Ok(SqlConn {
                api,
                db: handle as *mut c_void,
                target: target.to_string(),
            });
        }
        let Ok(path) = CString::new(target) else {
            return Err(format!("`{target}` is not a usable database path"));
        };
        let mut db: *mut c_void = std::ptr::null_mut();
        // READONLY is the contract, not a precaution: v1 refuses writes, and a
        // handle that cannot write refuses them at the one place that cannot be
        // forgotten.
        let rc = (api.open_v2)(
            path.as_ptr(),
            &raw mut db,
            sq::OPEN_READONLY,
            std::ptr::null(),
        );
        if rc != 0 || db.is_null() {
            let why = if db.is_null() {
                format!("`{target}` could not be opened (sqlite rc {rc})")
            } else {
                let msg = cstr(&api, db);
                (api.close)(db);
                format!("`{target}` could not be opened: {msg}")
            };
            return Err(why);
        }
        // MEASURED, and it changes what quoting means on this engine: SQLite
        // accepts a double-quoted name that resolves to no identifier as a STRING
        // LITERAL. `SELECT "naam" FROM "person"` on a table with a `name` column
        // returns the text `naam` for every row — no error, and the wrongest
        // possible answer, since the derived query quotes every identifier
        // (`sql_query.rs`) and a renamed column is exactly failure path 9.
        //
        // Turning the misfeature off is one call on the connection core owns, so
        // a name that resolves to nothing raises instead. Best effort: an SQLite
        // older than 3.29 does not know these options, and there the bind-time
        // schema check is the backstop.
        unsafe {
            (api.db_config)(db, sq::DQS_DML, 0, std::ptr::null::<c_int>());
            (api.db_config)(db, sq::DQS_DDL, 0, std::ptr::null::<c_int>());
        }
        table.insert(target.to_string(), db as usize);
        Ok(SqlConn {
            api,
            db,
            target: target.to_string(),
        })
    }

    /// Run `sql`, binding `args` positionally, and return every row.
    ///
    /// Rows come back whole rather than through a cursor: a keyed fault wants
    /// one row and a range wants the slice, and in both cases the caller is
    /// about to materialise all of them into the collection.
    ///
    /// # Errors
    /// When the statement does not prepare or does not run — a schema that
    /// drifted (a renamed column) surfaces here, which is why the message
    /// carries the engine's own text.
    pub fn query(&self, sql: &str, args: &[Cell]) -> Result<Vec<Row>, String> {
        let api = &self.api;
        let Ok(text) = CString::new(sql) else {
            return Err("the derived query contains a NUL byte".to_string());
        };
        let mut st: *mut c_void = std::ptr::null_mut();
        let rc = (api.prepare)(
            self.db,
            text.as_ptr(),
            -1,
            &raw mut st,
            std::ptr::null_mut(),
        );
        if rc != 0 || st.is_null() {
            return Err(format!("`{}`: {} [{sql}]", self.target, cstr(api, self.db)));
        }
        // Keep every CString alive until the statement is stepped: sqlite is
        // told to COPY (`SQLITE_TRANSIENT`, the pointer -1), but the pointer
        // still has to be valid for the duration of the bind call itself.
        let mut held = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let n = c_int::try_from(i + 1).unwrap_or(c_int::MAX);
            let ok = match a {
                Cell::Int(v) => (api.bind_int)(st, n, *v),
                Cell::Real(v) => (api.bind_double)(st, n, *v),
                Cell::Text(s) => {
                    let Ok(cs) = CString::new(s.as_str()) else {
                        (api.finalize)(st);
                        return Err("a key value contains a NUL byte".to_string());
                    };
                    let rc = (api.bind_text)(st, n, cs.as_ptr(), -1, transient());
                    held.push(cs);
                    rc
                }
            };
            if ok != 0 {
                let why = cstr(api, self.db);
                (api.finalize)(st);
                return Err(format!("`{}`: {why}", self.target));
            }
        }

        let mut rows = Vec::new();
        loop {
            let rc = (api.step)(st);
            if rc == sq::DONE {
                break;
            }
            if rc != sq::ROW {
                let why = cstr(api, self.db);
                (api.finalize)(st);
                return Err(format!("`{}`: {why} [{sql}]", self.target));
            }
            let n = (api.col_count)(st);
            let mut row: Row = Vec::with_capacity(n.max(0) as usize);
            for i in 0..n {
                row.push(match (api.col_type)(st, i) {
                    // NULL first: the whole point of asking the type is that
                    // `column_text` answers a null pointer for a SQL NULL and an
                    // empty string for `''`, and reading them the same way is the
                    // bug a binding exists to prevent.
                    sq::NULL_TYPE => None,
                    sq::INTEGER => Some(Cell::Int((api.col_int)(st, i))),
                    sq::FLOAT => Some(Cell::Real((api.col_double)(st, i))),
                    _ => {
                        let p = (api.col_text)(st, i);
                        if p.is_null() {
                            None
                        } else {
                            Some(Cell::Text(
                                unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned(),
                            ))
                        }
                    }
                });
            }
            rows.push(row);
        }
        (api.finalize)(st);
        Ok(rows)
    }
}

/// `SQLITE_TRANSIENT` — the pointer `-1`, which tells sqlite to COPY the bytes
/// before returning. That copy is what makes it correct to bind a Rust `CString`
/// that is dropped at the end of the call.
fn transient() -> *const c_void {
    usize::MAX as *const c_void
}

/// The engine's own last-error text for this connection.
fn cstr(api: &SqliteApi, db: *mut c_void) -> String {
    let p = (api.errmsg)(db);
    if p.is_null() {
        "unknown error".to_string()
    } else {
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}

/// Split a lazy source string into a driver and its target, or `None` when the
/// string does not name a database at all.
///
/// The prefix is what makes one `store_bind_lazy` serve both sources: a path is
/// an image, `sqlite:people.db` is a database, and nothing has to be declared
/// twice.
#[must_use]
pub fn driver_of(source: &str) -> Option<(Driver, &str)> {
    source
        .strip_prefix("sqlite:")
        .map(|target| (Driver::Sqlite, target))
}
