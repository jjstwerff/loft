// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! Database operations on stores
#![allow(dead_code)]

mod allocation;
mod format;
mod io;
mod search;
mod structures;
mod types;

pub use types::Type;

/// Store index reserved for compile-time constant data (vectors, long strings).
/// Always allocated during `State::new()`, locked before execution begins.
/// See `doc/claude/CONST_STORE.md` for the full design.
pub const CONST_STORE: u16 = 1;

use crate::keys::{Content, DbRef};
use crate::store::Store;
use std::collections::HashMap;
use std::fmt::{Debug, Formatter, Write as _};
use std::sync::{Arc, Mutex};
// P137: the `--html` build compiles for wasm32-unknown-unknown
// WITHOUT the `wasm` feature (the feature carries wasm-bindgen
// host bridges that `--html`'s hand-rolled JS runtime does not
// provide).  That leaves `std::time::Instant` on a target with no
// time source — calling `Instant::now()` panics, and the panic
// compiles to `(unreachable)` which was the root of every
// `--html loft_start` trap.  Use Instant only on non-wasm32
// targets; wasm32 (with or without the feature) tracks time in
// milliseconds through the host bridge.
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

/// Type alias for a native function callable from loft bytecode.
pub type Call = fn(&mut Stores, &mut DbRef);

/// Context injected into `Stores` by `State::execute()` so that native
/// functions such as `n_parallel_for_int` can access the interpreter's
/// bytecode, text segment, library, and compiled data for spawning workers.
///
/// All raw pointers are valid for the duration of the `execute()` call
/// that set them.
pub struct ParallelCtx {
    pub bytecode: *const Arc<Vec<u8>>,
    pub library: *const Arc<Vec<Call>>,
    pub data: *const crate::data::Data,
    /// Cached library index of `n_stack_trace`; `u16::MAX` = not found.
    /// Copied into worker `State::stack_trace_lib_nr` so workers can snapshot
    /// the call stack when `stack_trace()` is called (fix #92).
    pub stack_trace_lib_nr: u16,
}

// Safety: the pointed-to data lives for the duration of `State::execute()`,
// which is on the main thread and outlives all worker threads it spawns
// (workers are joined before execute() returns).
unsafe impl Send for ParallelCtx {}
unsafe impl Sync for ParallelCtx {}

/// TR1.4: snapshot of one local variable's runtime value, captured by
/// `State::static_call` for inclusion in a `StackFrame.variables` vector.
/// All fields are owned values — no raw pointers — so the snapshot is safe
/// to retain across native function boundaries.
#[derive(Debug, Clone)]
pub struct VarSnapshot {
    pub name: String,
    pub type_name: String,
    pub value: VarValueSnapshot,
}

/// Owned snapshot of a variable's typed runtime value.  Mirrors the loft
/// `ArgValue` enum so the native can populate `VarInfo.value` directly.
#[derive(Debug, Clone)]
pub enum VarValueSnapshot {
    Null,
    Bool(bool),
    Int(i32),
    Long(i64),
    Float(f64),
    Single(f32),
    Char(char),
    Text(String),
    Ref { store: i32, rec: i32, pos: i32 },
    Other(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    pub(self) content: u16,
    pub position: u16,
    pub default: Content,
    pub(self) other_indexes: Vec<u16>, // For now only fields on the same record
}

#[derive(Debug, Clone, PartialEq)]
pub enum Parts {
    Base,                              // One of the simple base types or text.
    Struct(Vec<Field>),                // The fields of this record.
    Enum(Vec<(u16, String)>),          // Enumerate type with possible values.
    EnumValue(u8, Vec<Field>),         // Enumerate value with actual value for typed structures.
    Byte(i32, bool),                   // start number and nullable flag
    Short(i32, bool),                  // start number and nullable flag
    Vector(u16),                       // The records are part of the vector
    Array(u16),                        // The array holds references for each record
    Sorted(u16, Vec<(u16, bool)>),     // Sorted vector on fields with an ascending flag
    Ordered(u16, Vec<(u16, bool)>),    // Sorted array on fields with an ascending flag
    Hash(u16, Vec<u16>), // A hash table, listing the field numbers that define its key
    Index(u16, Vec<(u16, bool)>, u16), // An index to a table, listing the key fields and the left field-nr
    Spacial(u16, Vec<u16>),            // A spacial index with the listed coordinate fields as a key
}

impl PartialEq for Content {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Content::Long(l), Content::Long(r)) => l == r,
            (Content::Float(l), Content::Float(r)) => l == r,
            (Content::Single(l), Content::Single(r)) => l == r,
            (Content::Str(s), Content::Str(o)) => s.str() == o.str(),
            _ => false,
        }
    }
}

impl Debug for Content {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Content::Long(l) => f.write_fmt(format_args!("Long({l})"))?,
            Content::Float(v) => f.write_fmt(format_args!("Float({v})"))?,
            Content::Single(s) => f.write_fmt(format_args!("Single({s})"))?,
            Content::Str(t) => {
                f.write_char('"')?;
                f.write_str(t.str())?;
                f.write_char('"')?;
            }
        }
        Ok(())
    }
}

pub struct Stores {
    pub types: Vec<Type>,
    pub names: HashMap<String, u16>,
    pub allocations: Vec<Store>,
    #[cfg(not(feature = "wasm"))]
    pub files: Vec<Option<std::fs::File>>,
    #[cfg(feature = "wasm")]
    pub files: Vec<()>,
    pub max: u16,
    /// S29 (P1-R4 M4-b): bitmap of free store slots — bit `i` is set when `allocations[i]`
    /// is free and eligible for reuse.  `database_named` finds the lowest set bit below `max`
    /// and reuses that slot instead of always growing `max`.  This eliminates the LIFO-order
    /// requirement on `free()` that the old cascade-based scan imposed.
    pub free_bits: Vec<u64>,
    /// Temporary strings produced by text-returning native functions.
    /// Cleared by `OpClearScratch` at statement boundaries.
    pub scratch: Vec<String>,
    /// Errors from the last `Type.parse()` call, read via `s#errors`.
    pub last_parse_errors: Vec<String>,
    /// Set by `State::execute()` to allow native functions to access the
    /// interpreter's bytecode, library, and compiled data during execution.
    pub parallel_ctx: Option<Box<ParallelCtx>>,
    /// Shared runtime logger.  Set by `main.rs` after the State is created.
    /// Cloned (Arc clone) into worker Stores so all threads share a single logger.
    pub logger: Option<Arc<Mutex<crate::logger::Logger>>>,
    /// Set to `true` when a loft `panic()` or failed `assert` fires in production mode
    /// (where the error is logged instead of aborting).  `main.rs` checks this after
    /// execution and exits with code 1 so shell scripts can detect failure.
    pub had_fatal: bool,
    /// Directory of the main source file being executed.
    /// Set by `main.rs` after parsing; used by `source_dir()` built-in.
    pub source_dir: String,
    /// FY.1: When true, the interpreter loop yields back to the caller.
    /// Set by `gl_swap_buffers` in WASM mode; cleared by `resume_frame`.
    pub frame_yield: bool,
    /// When true, assert() reports results (pass/fail) to `assert_results`
    /// instead of panicking on failure.  Used by the WASM playground.
    pub report_asserts: bool,
    /// Structured assert results: (passed, message, file, line).
    pub assert_results: Vec<(bool, String, String, u32)>,
    /// Script-level arguments (set by the CLI after parsing its own flags).
    /// When non-empty, `os_arguments()` returns these instead of raw `std::env::args`.
    pub user_args: Vec<String>,
    /// Monotonic timestamp captured at `Stores::new()`.  Used by `ticks()` to return
    /// microseconds elapsed since program start; cloned into worker Stores unchanged so
    /// all threads share the same reference point.
    #[cfg(not(target_arch = "wasm32"))]
    pub start_time: Instant,
    /// Under any wasm32 target (both the `wasm` feature's host-bridge
    /// build and the `--html` no-feature build): milliseconds since
    /// Unix epoch at program start.  `n_ticks` uses this plus the
    /// host-imported `time_ticks` to compute elapsed time without
    /// `std::time::Instant`.  See P137 for why we can't use Instant
    /// on wasm32 even without the `wasm` feature.
    #[cfg(target_arch = "wasm32")]
    pub start_time_ms: i64,
    /// TR1.3: snapshot of (`fn_name`, file, line) for each call frame.
    /// Populated by `State::static_call` when `n_stack_trace` is invoked.
    pub call_stack_snapshot: Vec<(String, String, u32)>,
    /// TR1.4: per-frame variable snapshot.  Outer Vec is parallel to
    /// `call_stack_snapshot` (one entry per frame); inner Vec is the live
    /// variables in that frame as `(name, type_name, ArgValueSnapshot)`.
    /// Populated alongside `call_stack_snapshot` in `State::static_call`.
    pub variables_snapshot: Vec<Vec<VarSnapshot>>,
    /// Native-code closure store. Maps lambda d_nr → closure DbRef.
    /// Set by `OpStoreClosure` (native) immediately before calling the lambda;
    /// read by `OpGetClosure` in the match-dispatch arm.
    pub closure_map: HashMap<u32, DbRef>,
}

impl Default for Stores {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for Stores {
    /// Clone the type-schema portion of a `Stores`.
    /// Runtime-only fields (`allocations`, `files`, `parallel_ctx`)
    /// are reset to empty/None because they are only valid during execution.
    fn clone(&self) -> Self {
        Self {
            types: self.types.clone(),
            names: self.names.clone(),
            allocations: Vec::new(),
            files: Vec::new(),
            max: self.max,
            free_bits: Vec::new(),
            scratch: Vec::new(),
            last_parse_errors: Vec::new(),
            parallel_ctx: None,
            logger: self.logger.clone(),
            had_fatal: false,
            source_dir: String::new(),
            frame_yield: false,
            report_asserts: false,
            assert_results: Vec::new(),
            user_args: self.user_args.clone(),
            #[cfg(not(target_arch = "wasm32"))]
            start_time: self.start_time,
            #[cfg(target_arch = "wasm32")]
            start_time_ms: self.start_time_ms,
            call_stack_snapshot: Vec::new(),
            variables_snapshot: Vec::new(),
            closure_map: HashMap::new(),
        }
    }
}

// Safety: `Content::Str` raw pointers in type metadata point into parse-time
// source strings that live for the program duration and are never mutated.
// Workers only read this metadata.  `Store` is already `unsafe impl Send`.
// `Sync` is additionally required so that `OnceLock<(Data, Stores)>` can be
// used as a process-wide static; the same invariant (read-only after parse)
// makes concurrent shared access safe.
unsafe impl Send for Stores {}
unsafe impl Sync for Stores {}

/// Type-level proof that a [`Stores`] was produced by [`Stores::clone_for_worker`]
/// and belongs to exactly one worker thread.
///
/// `WorkerStores` is `Send` (movable to a worker thread) but intentionally not
/// `Sync` (cannot be shared across threads).  The `PhantomData<*mut ()>` field
/// suppresses the auto-derived `Sync` implementation; the explicit `Send`
/// implementation restores send-ability.  This ensures that passing a worker
/// snapshot to `State::new_worker` at the call site is a compile-time guarantee
/// rather than a runtime convention.
pub struct WorkerStores {
    pub(crate) stores: Stores,
    _not_sync: std::marker::PhantomData<*mut ()>,
}

// SAFETY: each worker thread receives exclusive ownership of its WorkerStores.
// The inner Stores is a locked snapshot of main-thread data; workers never
// access the main thread's mutable state through this value.
unsafe impl Send for WorkerStores {}

impl WorkerStores {
    pub(crate) fn new(stores: Stores) -> Self {
        WorkerStores {
            stores,
            _not_sync: std::marker::PhantomData,
        }
    }
}

impl std::ops::Deref for WorkerStores {
    type Target = Stores;
    fn deref(&self) -> &Stores {
        &self.stores
    }
}

struct ParseKey {
    // The current line on the source data. Only relevant if that has a pretty print format.
    line: u32,
    // The position on the current line in utf-8 characters. We count zero width characters.
    line_pos: u32,
    // The current key: holds positions of key identifiers or vector steps when negative.
    current: Vec<i64>,
    // The current step on the key can decrease due to finished structures.
    step: u32,
}

fn parse_key(text: &str, pos: &mut usize, result: usize, key: &mut ParseKey) {
    if *pos >= result {
        return;
    }
    skip_empty(text, pos, key);
    if match_token(text, pos, b'[') {
        key.line_pos += 1;
        if key.step == key.current.len() as u32 {
            key.current.push(-1);
        } else {
            key.current[key.step as usize] = -1;
        }
        key.step += 1;
        skip_empty(text, pos, key);
        loop {
            parse_key(text, pos, result, key);
            if match_token(text, pos, b',') {
                key.current[key.step as usize] -= 1;
                key.line_pos += 1;
                skip_empty(text, pos, key);
            } else {
                break;
            }
        }
        if !match_token(text, pos, b']') {
            *pos = usize::MAX;
            return;
        }
        key.line_pos += 1;
        key.step -= 1;
    } else if match_token(text, pos, b'{') {
        key.line_pos += 1;
        if key.step == key.current.len() as u32 {
            key.current.push(*pos as i64);
        } else {
            key.current[key.step as usize] = *pos as i64;
        }
        key.step += 1;
        skip_empty(text, pos, key);
        loop {
            let mut val = String::new();
            match_identifier(text, pos, &mut val);
            key.line_pos += val.len() as u32;
            skip_empty(text, pos, key);
            if match_token(text, pos, b':') {
                key.line_pos += 1;
            } else {
                *pos = usize::MAX;
                return;
            }
            parse_key(text, pos, result, key);
            if match_token(text, pos, b',') {
                key.line_pos += 1;
            } else {
                break;
            }
            skip_empty(text, pos, key);
        }
        if !match_token(text, pos, b'}') {
            *pos = usize::MAX;
            return;
        }
        key.line_pos += 1;
        key.step -= 1;
    } else {
        let p = skip_float(text, pos);
        if p > *pos {
            *pos = p;
            return;
        }
        let mut val = String::new();
        // allow for constant strings
        if match_text(text, pos, &mut val) {
            return;
        }
        // allow for 'true', 'false', 'null', etc.
        if match_identifier(text, pos, &mut val) {
            return;
        }
        *pos = usize::MAX;
    }
}

fn skip_empty(text: &str, pos: &mut usize, key: &mut ParseKey) {
    let mut c = *pos;
    let bytes = text.as_bytes();
    while c < bytes.len() && (bytes[c] == b' ' || bytes[c] == b'\t' || bytes[c] == b'\n') {
        if bytes[c] == b'\n' {
            key.line += 1;
            key.line_pos = 0;
        } else {
            key.line_pos += 1;
        }
        c += 1;
        *pos = c;
    }
}

fn show_key(text: &str, key: &ParseKey) -> String {
    let mut result = format!("line {}:{} path:", key.line, key.line_pos);
    for k in 0..key.step {
        let p = key.current[k as usize];
        if p < 0 {
            write!(result, "[{}]", 1 - p).unwrap();
        } else {
            let mut pos = key.current[k as usize] as usize;
            let mut val = String::new();
            match_identifier(text, &mut pos, &mut val);
            if k > 0 {
                result += ".";
            }
            result += &val;
        }
    }
    result
}

fn match_token(text: &str, pos: &mut usize, token: u8) -> bool {
    if *pos < text.len() && text.as_bytes()[*pos] == token {
        *pos += 1;
        true
    } else {
        false
    }
}

fn match_identifier(text: &str, pos: &mut usize, value: &mut String) -> bool {
    let mut c = *pos;
    let bytes = text.as_bytes();
    if c < bytes.len()
        && ((bytes[c] >= b'a' && bytes[c] <= b'z')
            || bytes[c] >= b'A' && bytes[c] <= b'Z'
            || bytes[c] == b'_')
    {
        c += 1;
        while c < bytes.len()
            && ((bytes[c] >= b'0' && bytes[c] <= b'9')
                || (bytes[c] >= b'a' && bytes[c] <= b'z')
                || bytes[c] >= b'A' && bytes[c] <= b'Z'
                || bytes[c] == b'_')
        {
            c += 1;
        }
        *value = text[*pos..c].parse().unwrap();
        *pos = c;
        true
    } else {
        false
    }
}

fn skip_float(text: &str, pos: &mut usize) -> usize {
    let mut c = *pos;
    let bytes = text.as_bytes();
    if c < bytes.len() && bytes[c] == b'-' {
        c += 1;
    }
    while c < bytes.len()
        && ((bytes[c] >= b'0' && bytes[c] <= b'9') || bytes[c] == b'e' || bytes[c] == b'.')
    {
        c += 1;
        if c < bytes.len() && bytes[c - 1] == b'e' && bytes[c] == b'-' {
            c += 1;
        }
    }
    c
}

fn match_text(text: &str, pos: &mut usize, val: &mut String) -> bool {
    structures::match_text(text, pos, val)
}

#[allow(dead_code)]
impl Stores {
    #[must_use]
    pub fn new() -> Stores {
        let mut result = Stores {
            types: Vec::new(),
            names: HashMap::new(),
            allocations: Vec::new(),
            files: Vec::new(),
            max: 0,
            free_bits: Vec::new(),
            scratch: Vec::new(),
            last_parse_errors: Vec::new(),
            parallel_ctx: None,
            logger: None,
            had_fatal: false,
            source_dir: String::new(),
            frame_yield: false,
            report_asserts: false,
            assert_results: Vec::new(),
            user_args: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            start_time: Instant::now(),
            // P137: `Stores::new()` must not call `Instant::now()` or
            // `SystemTime::now()` on wasm32-unknown-unknown — both
            // trap as `(unreachable)` with no time source.  The
            // `--html` build (wasm32, no `wasm` feature) uses 0 as
            // the epoch stub; the full `wasm` feature build routes
            // through the host bridge.
            #[cfg(all(target_arch = "wasm32", feature = "wasm"))]
            start_time_ms: crate::wasm::host_time_now(),
            #[cfg(all(target_arch = "wasm32", not(feature = "wasm")))]
            start_time_ms: 0,
            call_stack_snapshot: Vec::new(),
            variables_snapshot: Vec::new(),
            closure_map: HashMap::new(),
        };
        result.base_type("integer", 4); // 0
        result.base_type("long", 8); // 1
        result.base_type("single", 4); // 2
        result.base_type("float", 8); // 3
        result.base_type("boolean", 1); // 4
        result.base_type("text", 4); // 5
        result.base_type("character", 4); // 6
        result
    }

    #[must_use]
    pub fn get<T>(&mut self, stack: &mut DbRef) -> &T {
        debug_assert!(
            stack.pos >= size_of::<T>() as u32,
            "Stack underflow in get<{}>: stack.pos={} but need {} bytes",
            std::any::type_name::<T>(),
            stack.pos,
            size_of::<T>(),
        );
        stack.pos -= size_of::<T>() as u32;
        self.store(stack).addr::<T>(stack.rec, stack.pos)
    }

    pub fn put<T>(&mut self, stack: &mut DbRef, val: T) {
        let m = self.store_mut(stack).addr_mut::<T>(stack.rec, stack.pos);
        *m = val;
        stack.pos += size_of::<T>() as u32;
    }

    /// Look up a type by index, panicking with a diagnostic if the index is out of range.
    ///
    /// # Panics
    /// Panics if `nr` is out of range for the types table.
    #[must_use]
    pub fn get_type(&self, nr: u16) -> &Type {
        self.types.get(nr as usize).unwrap_or_else(|| {
            panic!(
                "type index {} out of range (total: {})",
                nr,
                self.types.len()
            )
        })
    }
}

pub struct ShowDb<'a> {
    pub stores: &'a Stores,
    pub store: u16,
    pub rec: u32,
    pub pos: u32,
    pub known_type: u16,
    pub pretty: bool,
    pub json: bool,
}

/// Structured debug dump with store/record references, depth and element limits.
/// Used for `tests/dumps/*.txt` diagnostics and `LOFT_LOG` execution trace.
pub struct DumpDb<'a> {
    pub stores: &'a Stores,
    pub store: u16,
    pub rec: u32,
    pub pos: u32,
    pub known_type: u16,
    /// Maximum nesting depth (0 = just the value, 1 = one level of fields, etc.)
    pub max_depth: u16,
    /// Maximum number of array/vector elements to show before `...`
    pub max_elements: u16,
    /// When true, output stays on a single line (spaces instead of newlines).
    pub compact: bool,
}

/// `get_type()` with an out-of-range index must panic with a helpful message.
#[test]
#[should_panic(expected = "type index 999 out of range")]
fn get_type_out_of_range_panics() {
    let stores = Stores::new();
    let _ = stores.get_type(999);
}

// These values are for amd64 or arm64 systems.
// It's not possible to test these continuously as these will fail on 32-bit systems.
#[test]
fn sizes() {
    /*
    assert_eq!(size_of::<DbRef>(), 12);
    assert_eq!(size_of::<String>(), 24);
    assert_eq!(size_of::<&str>(), 16);
    */
}
