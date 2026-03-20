// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! Database operations on stores
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]
#![allow(dead_code)]

mod allocation;
mod format;
mod io;
mod search;
mod structures;
mod types;

pub use types::Type;

use crate::keys::{Content, DbRef};
use crate::store::Store;
use std::collections::HashMap;
use std::fmt::{Debug, Formatter, Write as _};
use std::sync::{Arc, Mutex};
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
    pub text_code: *const Arc<Vec<u8>>,
    pub library: *const Arc<Vec<Call>>,
    pub data: *const crate::data::Data,
}

// Safety: the pointed-to data lives for the duration of `State::execute()`,
// which is on the main thread and outlives all worker threads it spawns
// (workers are joined before execute() returns).
unsafe impl Send for ParallelCtx {}
unsafe impl Sync for ParallelCtx {}

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
    pub files: Vec<Option<std::fs::File>>,
    pub max: u16,
    /// Set by `State::execute()` to allow native functions to access the
    /// interpreter's bytecode, library, and compiled data during execution.
    pub parallel_ctx: Option<Box<ParallelCtx>>,
    /// Shared runtime logger.  Set by `main.rs` after the State is created.
    /// Cloned (Arc clone) into worker Stores so all threads share a single logger.
    pub logger: Option<Arc<Mutex<crate::logger::Logger>>>,
    /// Temporary strings produced by native functions (e.g. `to_uppercase`, `replace`).
    /// Native functions that create new owned strings push them here and return a
    /// `Str` (raw pointer + length) pointing into the stored data.  The strings live
    /// for the lifetime of the interpreter run, which is safe for short programs and
    /// bounded-size test suites.
    pub scratch: Vec<String>,
    /// Set to `true` when a loft `panic()` or failed `assert` fires in production mode
    /// (where the error is logged instead of aborting).  `main.rs` checks this after
    /// execution and exits with code 1 so shell scripts can detect failure.
    pub had_fatal: bool,
    /// Monotonic timestamp captured at `Stores::new()`.  Used by `ticks()` to return
    /// microseconds elapsed since program start; cloned into worker Stores unchanged so
    /// all threads share the same reference point.
    pub start_time: Instant,
}

impl Default for Stores {
    fn default() -> Self {
        Self::new()
    }
}

// Safety: `Content::Str` raw pointers in type metadata point into parse-time
// source strings that live for the program duration and are never mutated.
// Workers only read this metadata.  `Store` is already `unsafe impl Send`.
unsafe impl Send for Stores {}

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
            parallel_ctx: None,
            logger: None,
            scratch: Vec::new(),
            had_fatal: false,
            start_time: Instant::now(),
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
        stack.pos -= size_of::<T>() as u32;
        self.store(stack).addr::<T>(stack.rec, stack.pos)
    }

    pub fn put<T>(&mut self, stack: &mut DbRef, val: T) {
        let m = self.store_mut(stack).addr_mut::<T>(stack.rec, stack.pos);
        *m = val;
        stack.pos += size_of::<T>() as u32;
    }

    /// Look up a type by index, panicking with a diagnostic if the index is out of range.
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

/// S6-65: get_type() with an out-of-range index must panic with a helpful message.
#[test]
#[ignore]
#[should_panic(expected = "type index 999 out of range")]
fn get_type_out_of_range_panics() {
    let stores = Stores::new();
    stores.get_type(999);
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
