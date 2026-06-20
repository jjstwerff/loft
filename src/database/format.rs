// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! Display/debug formatting: `show`, `show_value`, `dump` functions.

use crate::database::{Field, Parts, ShowDb, Stores};
use crate::keys::{self, DbRef};
use crate::store::Store;
use crate::vector;
use std::collections::BTreeMap;
use std::fmt::{Debug, Formatter, Write as _};

/// Render a walker / unified-parser failure as `"line N:M path:X"`.
///
/// The path is collected by [`Stores::walk_parsed_into`] as it
/// descends, and the (line, col) pair comes from
/// [`crate::json::line_col_of`].  Asserted-on directly by
/// `tests/data_structures.rs::record` (e.g. `"line 1:7 path:blame"`).
fn format_walk_err(text: &str, at: usize, path: &[String]) -> String {
    let (line, col) = crate::json::line_col_of(text, at);
    let mut out = format!("line {line}:{col} path:");
    for (i, seg) in path.iter().enumerate() {
        if i > 0 && !seg.starts_with('[') {
            out.push('.');
        }
        out.push_str(seg);
    }
    out
}

#[allow(dead_code)]
impl Stores {
    #[must_use]
    pub fn rec(&self, db: &DbRef, tp: u16) -> String {
        let mut res = String::new();
        self.show(&mut res, db, tp, false);
        res
    }

    pub fn dump(&self, db: &DbRef, tp: u16) {
        let mut check = String::new();
        self.show(&mut check, db, tp, true);
        println!("data: {check}");
    }

    pub fn show(&self, s: &mut String, db: &DbRef, tp: u16, pretty: bool) {
        self.valid(db);
        ShowDb {
            stores: self,
            store: db.store_nr,
            rec: db.rec,
            pos: db.pos,
            known_type: tp,
            pretty,
            json: false,
            loft: false,
            dump: false,
            compact: false,
            max_depth: u16::MAX,
            max_elements: u16::MAX,
        }
        .write(s, 0);
    }

    /// Serialise a record to **native loft source** — `TypeName{field: value}`,
    /// `Enum.Variant`, quoted+escaped text, forced-decimal floats, `[…]` vectors.
    /// Reuses the same `ShowDb` schema walk as [`show`](Self::show) /
    /// [`show_json`](Self::show_json) in its `loft` mode.  The output re-parses
    /// through both the database parser (`Stores::parse`) and the loft language
    /// parser, so a value round-trips to itself — the own-format serializer for
    /// REPL value-snapshot and live data migration (@PLN12 REPL.X).
    pub fn show_loft(&self, s: &mut String, db: &DbRef, tp: u16) {
        self.valid(db);
        ShowDb {
            stores: self,
            store: db.store_nr,
            rec: db.rec,
            pos: db.pos,
            known_type: tp,
            pretty: false,
            json: false,
            loft: true,
            dump: false,
            compact: false,
            max_depth: u16::MAX,
            max_elements: u16::MAX,
        }
        .write(s, 0);
    }

    /// `show_loft` bounded to `max_depth` nesting levels and `max_elements` per vector —
    /// the clean re-parseable literal, but truncated (`{…}` / `…+N`) so a big struct
    /// doesn't flood.  Backs the debugger's variables panel (a glance, not the whole heap);
    /// the full value is still one `eval` away.  `u16::MAX` for either = unlimited.
    pub fn show_loft_bounded(
        &self,
        s: &mut String,
        db: &DbRef,
        tp: u16,
        max_depth: u16,
        max_elements: u16,
    ) {
        self.valid(db);
        ShowDb {
            stores: self,
            store: db.store_nr,
            rec: db.rec,
            pos: db.pos,
            known_type: tp,
            pretty: false,
            json: false,
            loft: true,
            dump: false,
            compact: false,
            max_depth,
            max_elements,
        }
        .write(s, 0);
    }

    /// Serialise a record to RFC 8259 JSON text.  Backs `T.to_json()`
    /// (Q3 second half, P54).  Reuses the schema-walking machinery in
    /// `ShowDb` (every `Parts::*` arm already implemented) but engages
    /// the `json: true` formatting branches: text strings are JSON-
    /// escaped, field names are quoted, struct-enum variants are
    /// wrapped as `{"VariantName": {fields}}`, and `JsonValue` fields
    /// render their existing subtree verbatim instead of as the
    /// generic enum-variant shape.  `pretty: false` produces canonical
    /// (single-line, no spaces) output; `pretty: true` produces the
    /// 2-space-indent multi-line form mirroring `to_json_pretty`.
    pub fn show_json(&self, s: &mut String, db: &DbRef, tp: u16, pretty: bool) {
        self.valid(db);
        ShowDb {
            stores: self,
            store: db.store_nr,
            rec: db.rec,
            pos: db.pos,
            known_type: tp,
            pretty,
            json: true,
            loft: false,
            dump: false,
            compact: false,
            max_depth: u16::MAX,
            max_elements: u16::MAX,
        }
        .write(s, 0);
    }

    /**
    Get the Json-path inspired path to a record.
    # Panics
    When this path cannot be detected correctly.
    */
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn path(&self, db: &DbRef, tp: u16) -> String {
        if db.rec == 1 {
            return "/".to_string();
        }
        let p_rec = self.store(db).get_u32_raw(db.rec, 4);
        let p_tp = if self.types[tp as usize].parents.is_empty()
            || self.types[tp as usize].parents.len() > 1
        {
            self.store(db).get_short(p_rec, 8, 0) as u16
        } else {
            *self.types[tp as usize].parents.iter().next().unwrap()
        };
        let parent = DbRef {
            store_nr: db.store_nr,
            rec: p_rec,
            pos: 8,
        };
        let mut res = self.path(&parent, p_tp);
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
            &self.types[p_tp as usize].parts
        {
            for f in fields {
                let f_tp = &self.types[f.content as usize];
                // TODO this for now assumes that the child is linked only once.
                if f_tp.contains(tp) {
                    res += &f.name;
                    res += "[";
                    if f_tp.keys.is_empty() {
                        let data = DbRef {
                            store_nr: db.store_nr,
                            rec: db.rec,
                            pos: 8 + u32::from(f.position),
                        };
                        let mut pos = i32::MAX;
                        let mut count = 0;
                        loop {
                            vector::vector_next(&data, &mut pos, f_tp.size, &self.allocations);
                            if pos == i32::MAX {
                                res += "?";
                                break;
                            }
                            let rec = self.store(db).get_u32_raw(data.rec, data.pos);
                            if rec == db.rec {
                                write!(res, "{count}").unwrap();
                                break;
                            }
                            count += 1;
                        }
                    } else {
                        for (c_nr, c) in keys::get_key(db, &self.allocations, &f_tp.keys)
                            .iter()
                            .enumerate()
                        {
                            if c_nr > 0 {
                                res += ",";
                            }
                            write!(res, "{c}").unwrap();
                        }
                    }
                    res += "]";
                    break;
                }
                // If the field is an embedded sub-struct, check one level deeper:
                // the child type `tp` may live inside a collection that belongs to that sub-struct.
                if let Parts::Struct(sub_fields) | Parts::EnumValue(_, sub_fields) =
                    &self.types[f.content as usize].parts.clone()
                {
                    for sf in sub_fields {
                        let sf_tp = &self.types[sf.content as usize];
                        if sf_tp.contains(tp) {
                            // Build path via the sub-struct field name, then the inner field name.
                            res += &f.name;
                            res += ".";
                            res += &sf.name;
                            res += "[";
                            if sf_tp.keys.is_empty() {
                                let sub_data = DbRef {
                                    store_nr: db.store_nr,
                                    rec: db.rec,
                                    pos: 8 + u32::from(f.position) + u32::from(sf.position),
                                };
                                let mut pos = i32::MAX;
                                let mut count = 0;
                                loop {
                                    vector::vector_next(
                                        &sub_data,
                                        &mut pos,
                                        sf_tp.size,
                                        &self.allocations,
                                    );
                                    if pos == i32::MAX {
                                        res += "?";
                                        break;
                                    }
                                    let rec =
                                        self.store(db).get_u32_raw(sub_data.rec, sub_data.pos);
                                    if rec == db.rec {
                                        write!(res, "{count}").unwrap();
                                        break;
                                    }
                                    count += 1;
                                }
                            } else {
                                for (c_nr, c) in keys::get_key(db, &self.allocations, &sf_tp.keys)
                                    .iter()
                                    .enumerate()
                                {
                                    if c_nr > 0 {
                                        res += ",";
                                    }
                                    write!(res, "{c}").unwrap();
                                }
                            }
                            res += "]";
                            break;
                        }
                    }
                }
            }
        }
        res
    }

    /// Parse the content of a string into an existing record.
    /// Returns `None` on success, or `Some(error_path)` on failure.
    /// The error path is a human-readable string like `"line 1:15 path:items[2].name"`.
    ///
    /// Routes through the unified
    /// [`crate::json::parse_with(text, Dialect::Lenient)`] +
    /// schema-driven [`Stores::walk_parsed_into`] walker.  Both
    /// syntax-level errors (from the parser) and schema / shape
    /// mismatches (from the walker) feed into the same
    /// `"line N:M path:X"` shape via [`format_walk_err`], so
    /// callers see one consistent format regardless of where the
    /// failure originated.
    pub fn parse(&mut self, text: &str, tp: u16, result: &DbRef) -> Option<String> {
        self.try_parse_unified(text, tp, result).err()
    }

    // Used for testing, returns the interpreted data or the error path on problems.
    pub fn parse_message(&mut self, text: &str, tp: u16) -> String {
        let db = self.database(u32::from(self.types[tp as usize].size));
        self.store_mut(&db).set_u32_raw(db.rec, 4, u32::from(tp));
        match self.try_parse_unified(text, tp, &db) {
            Ok(()) => {
                let mut s = String::new();
                self.show(&mut s, &db, tp, false);
                s
            }
            Err(msg) => msg,
        }
    }

    /// Run the unified parse-then-walk path and translate any
    /// failure into the user-visible `"line N:M path:X"` shape.
    ///
    /// The unified parser ([`crate::json::parse_with`]) handles
    /// syntax-level errors (returns `ParseError` with byte offset),
    /// and the schema walker ([`Stores::walk_parsed_into`]) handles
    /// shape / type mismatches against the loft type definition
    /// (returns `WalkErr` with byte offset + dotted path).
    /// Both feed into the same `"line N:M path:X"` format that
    /// `tests/data_structures.rs::record` asserts.
    fn try_parse_unified(&mut self, text: &str, tp: u16, result: &DbRef) -> Result<(), String> {
        let parsed = crate::json::parse_with(text, crate::json::Dialect::Lenient)
            .map_err(|e| format_walk_err(text, e.byte_offset, &[]))?;
        let mut path: Vec<String> = Vec::new();
        self.walk_parsed_into(&parsed, tp, tp, u16::MAX, result, &mut path, 0)
            .map_err(|e| format_walk_err(text, e.at, &e.path))
    }

    /**
    Get the command line arguments into a vector
    # Panics
    When the OS provided incorrect arguments (non utf8 tokens inside it)
    */
    #[must_use]
    pub fn os_arguments(&mut self) -> DbRef {
        // `user_args` is the authoritative curated list of
        // script-level args; an empty one is a correct result.  We
        // never fall back to `std::env::args_os()`, which would leak
        // the binary path + loft CLI flags.
        let args = self.user_args.clone();
        self.text_vector(&args)
    }

    /// Build a `vector<text>` from an explicit string slice.
    #[must_use]
    pub fn text_vector(&mut self, args: &[String]) -> DbRef {
        let vec = self.database(4);
        self.store_mut(&vec).set_u32_raw(vec.rec, vec.pos, 0);
        for v in args {
            let elm = vector::vector_append(&vec, 4, &mut self.allocations);
            let s = self.store_mut(&vec).set_str(v.as_str());
            self.store_mut(&vec).set_u32_raw(elm.rec, elm.pos, s);
            vector::vector_finish(&vec, &mut self.allocations);
        }
        vec
    }

    /**
    Get all environment variables into a vector
    # Panics
    When the OS provided incorrect variable names (non utf8 tokens inside it)
    */
    #[must_use]
    pub fn os_variables(&mut self) -> DbRef {
        let elm = self.name("Variable");
        let size = u32::from(self.size(elm));
        let vec = self.database(size);
        self.store_mut(&vec).set_u32_raw(vec.rec, vec.pos, 0);
        #[cfg(not(feature = "wasm"))]
        for t in std::env::vars_os() {
            let name = t.0.to_str().unwrap();
            let value = t.1.to_str().unwrap();
            let elm = vector::vector_append(&vec, size, &mut self.allocations);
            let n = self.store_mut(&vec).set_str(name);
            let v = self.store_mut(&vec).set_str(value);
            self.store_mut(&vec).set_u32_raw(elm.rec, elm.pos, n);
            self.store_mut(&vec).set_u32_raw(elm.rec, elm.pos + 4, v);
            vector::vector_finish(&vec, &mut self.allocations);
        }
        vec
    }

    /**
    Get the value of an environment variable as an owned `String` ("" if unset).

    @PLN10 (Phase 2): returns owned `String` instead of a scratch-backed `Str`.
    The interpreter caller (`n_env_variable`) and its dest-passing variant own the
    String (push to a dest / scratch fallback); the native `#rust` template
    bridges `String` → `Str` via `Deref` (the @P304 path, like `to_lowercase`).
    Always non-null (empty for an unset variable).
    */
    #[cfg(not(feature = "wasm"))]
    #[must_use]
    pub fn os_variable(&mut self, name: &str) -> String {
        std::env::var_os(name)
            .and_then(|s| s.into_string().ok())
            .unwrap_or_default()
    }

    /**
    Get the value of an environment variable (WASM stub — always returns empty).
    */
    #[cfg(feature = "wasm")]
    #[must_use]
    pub fn os_variable(&mut self, name: &str) -> String {
        crate::wasm::host_env_variable(name)
    }

    /**
    Get the current directory
    # Panics
    When the OS provided incorrect variable values (non utf8 tokens inside it)
    */
    #[must_use]
    pub fn os_directory(s: &mut String) -> crate::keys::Str {
        s.clear();
        #[cfg(not(feature = "wasm"))]
        if let Ok(v) = std::env::current_dir() {
            *s += v.to_str().unwrap();
        }
        #[cfg(feature = "wasm")]
        {
            *s = crate::wasm::host_fs_cwd();
        }
        crate::keys::Str::new(s)
    }

    /**
    Get home directory
    # Panics
    When the OS provided incorrect variable values (non utf8 tokens inside it)
    */
    #[must_use]
    pub fn os_home(s: &mut String) -> crate::keys::Str {
        s.clear();
        #[cfg(not(feature = "wasm"))]
        if let Some(v) = dirs::home_dir() {
            *s += v.to_str().unwrap();
        }
        #[cfg(feature = "wasm")]
        {
            *s = crate::wasm::host_fs_user_dir();
        }
        crate::keys::Str::new(s)
    }

    /**
    Get the executable directory
    # Panics
    When the OS provided incorrect variable values (non utf8 tokens inside it)
    */
    #[must_use]
    pub fn os_executable(s: &mut String) -> crate::keys::Str {
        s.clear();
        #[cfg(not(feature = "wasm"))]
        if let Ok(v) = std::env::current_exe() {
            *s += v.to_str().unwrap();
        }
        #[cfg(feature = "wasm")]
        {
            *s = crate::wasm::host_fs_program_dir();
        }
        crate::keys::Str::new(s)
    }

    /// Native-codegen variant of `os_directory` that returns an owned `String`.
    /// Used by the `--native` backend where a scratch-buffer `&mut String` is not available.
    ///
    /// # Panics
    /// Panics if the current directory path contains non-UTF-8 characters.
    #[must_use]
    pub fn os_directory_native() -> String {
        #[cfg(not(feature = "wasm"))]
        {
            let mut s = String::new();
            if let Ok(v) = std::env::current_dir() {
                s += v.to_str().unwrap();
            }
            s
        }
        #[cfg(feature = "wasm")]
        crate::wasm::host_fs_cwd()
    }

    /// Return the byte at position `idx` (0..len) as i64 0-255.
    /// Out-of-bounds (idx < 0 or idx >= len) returns 0 — same
    /// neutral value `text_character` uses for OOB.  Unlike
    /// `text_character` which walks back through UTF-8
    /// continuation bytes and decodes a codepoint, this is a
    /// pure O(1) byte read.  Use for ASCII-heavy scanning hot
    /// paths (tokenisers, regex-like scanners) where the UTF-8
    /// decode is wasted work — every non-ASCII byte still
    /// returns a valid 0-255 number; the caller compares against
    /// ASCII constants so it doesn't matter what the byte means.
    #[must_use]
    pub fn text_byte_at_native(s: &str, idx: i64) -> i64 {
        let bytes = s.as_bytes();
        let len = bytes.len() as i64;
        let i = if idx < 0 { idx + len } else { idx };
        if i < 0 || i >= len {
            return 0;
        }
        i64::from(bytes[i as usize])
    }

    /// Build a `text` from the raw bytes of a `vector<u8>` — the inverse of
    /// `byte_at`.  Decodes the bytes as UTF-8; returns the decoded owned
    /// `String` so every binary decoder (CBOR text, HPKE byte composition)
    /// can turn an assembled byte buffer back into text.
    ///
    /// `bytes` is the argument `DbRef` (it points at the OWNING field, the
    /// same shape `n_json_array` reads): the inner vector record is read from
    /// `(bytes.rec, bytes.pos)`, the element count lives at offset 4 of that
    /// record, and the payload starts at offset 8 (`Store::buffer`) — the
    /// `vector::alloc_vector_from_bytes` layout.
    ///
    /// Invalid UTF-8 returns the empty string rather than panicking — the
    /// stdlib convention for a loft-safe primitive (`text_byte_at_native`
    /// returns 0 for an out-of-bounds read for the same reason).  A caller
    /// that needs to distinguish "empty input" from "invalid bytes" should
    /// validate before decoding.
    #[must_use]
    pub fn text_from_bytes_native(&mut self, bytes: DbRef) -> String {
        let length = vector::length_vector(&bytes, &self.allocations);
        if bytes.rec == 0 || bytes.pos == 0 || length == 0 {
            return String::new();
        }
        let store = self.store_mut(&bytes);
        let vec_rec = store.get_u32_raw(bytes.rec, bytes.pos);
        if vec_rec == 0 {
            return String::new();
        }
        // `buffer` returns the payload slice starting at offset 8; the live
        // length (offset 4) bounds it — trailing capacity slack is ignored.
        let raw = &store.buffer(vec_rec)[..length as usize];
        String::from_utf8(raw.to_vec()).unwrap_or_default()
    }

    /// Modification time of `path` as Unix epoch SECONDS (i64).
    /// Returns 0 on missing file, IO error, or pre-epoch dates.
    /// SECONDS not milliseconds — matches scan.sh's `stat -c %Y`
    /// / `stat -f %m` semantics so date-window filters
    /// (plans_recent's 60-day cutoff) get the same boundary
    /// behaviour as the bash port.  Use `ymd_days_ago(N)` for
    /// the cutoff and convert the seconds-since-epoch to
    /// YYYY-MM-DD for the lexicographic compare.
    #[must_use]
    pub fn os_mtime_native(path: &str) -> i64 {
        std::fs::metadata(path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs() as i64)
    }

    /// `YYYY-MM-DD` of today minus `days`, UTC.  Wraps the
    /// `days_to_ymd` algorithm in `src/logger.rs`.  Reused by the
    /// loft `ymd_days_ago(days)` builtin (interp + native via the
    /// `#rust` template).  Negative `days` clamps to today.
    #[must_use]
    pub fn ymd_days_ago_native(days: i64) -> String {
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let today_days = now_secs / 86_400;
        let target_days = today_days.saturating_sub(days.max(0) as u64);
        let (y, m, d) = crate::logger::days_to_ymd(target_days);
        format!("{y:04}-{m:02}-{d:02}")
    }

    /// Human-readable snapshot of every LIVE store's internal memory
    /// utilisation — total capacity vs actual claimed data vs free space,
    /// record / free-block counts, and the largest stores by capacity
    /// with their creation site and type.  Exposed to loft as
    /// `store_memory()` for diagnosing memory growth in a running program.
    ///
    /// Each per-store line shows `bc:<created_at>` — the bytecode position
    /// where the store was allocated.  On the interpreter that maps back
    /// to source via the `LOFT_LOG=static` bytecode dump; on `--native`
    /// there are no bytecode positions so it reads `bc:0` and only the
    /// `type` name identifies the origin.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn memory_report(&self) -> String {
        let mut live = 0u32;
        let (mut cap, mut data, mut free, mut recs, mut free_blk, mut mergeable) =
            (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
        // (store index, usage, created_at bytecode pos, known_type)
        let mut rows: Vec<(usize, crate::store::StoreUsage, u32, u16)> = Vec::new();
        for (i, s) in self.allocations.iter().enumerate() {
            if s.free {
                continue;
            }
            let u = s.usage();
            live += 1;
            cap += u64::from(u.capacity_words);
            data += u64::from(u.claimed_words);
            free += u64::from(u.free_words);
            recs += u64::from(u.claimed_count);
            free_blk += u64::from(u.free_count);
            mergeable += u64::from(u.mergeable_free_pairs);
            rows.push((i, u, s.created_at, s.known_type));
        }
        rows.sort_by_key(|r| std::cmp::Reverse(r.1.capacity_words));
        let mb = |w: u64| (w as f64) * 8.0 / 1_048_576.0;
        let pct = |a: u64, b: u64| {
            if b == 0 {
                0.0
            } else {
                100.0 * (a as f64) / (b as f64)
            }
        };
        let mut out = format!(
            "stores: {live} live | cap {:.2} MB | data {:.2} MB ({:.0}%) | free {:.2} MB | records {recs} | free-blocks {free_blk} | mergeable-pairs {mergeable}",
            mb(cap),
            mb(data),
            pct(data, cap),
            mb(free)
        );
        use std::fmt::Write as _;
        for (i, u, created_at, kt) in rows.iter().take(8) {
            let tname = if (*kt as usize) < self.types.len() {
                self.types[*kt as usize].name.clone()
            } else {
                "?".to_string()
            };
            let _ = write!(
                out,
                "\n  #{i:<5} {:>8.3} MB  used {:>3.0}%  recs {:<5} free-blk {:<4} mergeable {:<4} largest-free {:<7}w  type {tname} bc:{created_at}",
                mb(u64::from(u.capacity_words)),
                u.used_pct(),
                u.claimed_count,
                u.free_count,
                u.mergeable_free_pairs,
                u.largest_free_words,
            );
        }
        out
    }

    /// Native-codegen variant of `os_home` that returns an owned `String`.
    ///
    /// # Panics
    /// Panics if the home directory path contains non-UTF-8 characters.
    #[must_use]
    pub fn os_home_native() -> String {
        #[cfg(not(feature = "wasm"))]
        {
            let mut s = String::new();
            if let Some(v) = dirs::home_dir() {
                s += v.to_str().unwrap();
            }
            s
        }
        #[cfg(feature = "wasm")]
        crate::wasm::host_fs_user_dir()
    }

    /// Native-codegen variant of `os_executable` that returns an owned `String`.
    ///
    /// # Panics
    /// Panics if the executable path contains non-UTF-8 characters.
    #[must_use]
    pub fn os_executable_native() -> String {
        #[cfg(not(feature = "wasm"))]
        {
            let mut s = String::new();
            if let Ok(v) = std::env::current_exe() {
                s += v.to_str().unwrap();
            }
            s
        }
        #[cfg(feature = "wasm")]
        crate::wasm::host_fs_program_dir()
    }

    /// Native backend for `source_dir()` (`default/03_text.loft`'s
    /// `#rust"Stores::source_dir_native()"` template).
    ///
    /// #255 / @PLN9 Phase 1: under `--native` the program's "own directory" is
    /// the **executable's directory** for a standalone bundle (a compiled
    /// binary has no source tree, so the binary's location is the
    /// program-relative anchor) — but in DRIVER mode the executable is a
    /// generated artifact in the cache/tmp dir, so the driver hands the real
    /// program dir down via `LOFT_SOURCE_DIR`, which wins when set.  This
    /// mirrors `current_exe()` use in `native_utils::loft_lib_dir_for`.
    /// Returns "" only when `current_exe()` is unavailable (e.g. some
    /// sandboxed wasm hosts) — callers treat empty as "no anchor, fall back
    /// to cwd".
    #[must_use]
    pub fn source_dir_native() -> String {
        // #255 / @PLN9 Phase 1w: under wasm there is no executable path
        // (`current_exe()` is unreliable / empty under WASI).  The program's
        // effective "own directory" is the host's working directory — the WASI
        // preopen under wasmtime, or the asset base the browser host serves from
        // — which is also where relative file ops already resolve.  Under the
        // browser (no filesystem) `current_dir()` returns Err → "", and an empty
        // anchor makes `resolve_path` fall back to passthrough, letting the host
        // bridge resolve — the correct behaviour there.
        #[cfg(target_arch = "wasm32")]
        {
            return std::env::current_dir()
                .ok()
                .map(|d| d.to_string_lossy().into_owned())
                .unwrap_or_default();
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            // Driver mode: the executable is a generated artifact in the
            // cache/tmp dir — ITS dir is never where the program's assets
            // live.  The driver hands the real program dir down at spawn
            // (`LOFT_SOURCE_DIR`); a standalone bundle runs without the env
            // and keeps the executable-dir anchor.
            if let Ok(d) = std::env::var("LOFT_SOURCE_DIR") {
                return d;
            }
            std::env::current_exe()
                .ok()
                .as_deref()
                .and_then(std::path::Path::parent)
                .map(|d| d.to_string_lossy().into_owned())
                .unwrap_or_default()
        }
    }
}

impl Debug for ShowDb<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "({},{}):{}({})",
            self.rec, self.pos, self.stores.types[self.known_type as usize].name, self.known_type
        )
    }
}

impl ShowDb<'_> {
    fn store(&self) -> &Store {
        let r = DbRef {
            store_nr: self.store,
            rec: 0,
            pos: 0,
        };
        self.stores.store(&r)
    }

    /**
    Write data from the database into String s.
    # Panics
    When the database is not correct.
    */
    pub fn write(&self, s: &mut String, indent: u16) {
        if self.dump {
            // Trace form (`#store.rec` references + truncation) — its own emission rules.
            self.write_dump(s, indent);
            return;
        }
        if self.rec == 0 {
            write!(s, "null").unwrap();
            return;
        }
        if self.known_type == 0 {
            write!(s, "{}", self.store().get_int(self.rec, self.pos)).unwrap();
        } else if self.known_type == 1 {
            write!(s, "{}", self.store().get_long(self.rec, self.pos)).unwrap();
        } else if self.known_type == 2 {
            let v = self.store().get_single(self.rec, self.pos);
            if self.loft {
                s.push_str(&ensure_decimal(&format!("{v}")));
            } else {
                write!(s, "{v}").unwrap();
            }
        } else if self.known_type == 3 {
            let v = self.store().get_float(self.rec, self.pos);
            if self.loft {
                s.push_str(&ensure_decimal(&format!("{v}")));
            } else {
                write!(s, "{v}").unwrap();
            }
        } else if self.known_type == 4 {
            // 255 = @PLN17's three-state-boolean null sentinel (C73); inert for two-state.
            s.push_str(match self.store().get_byte(self.rec, self.pos, 0) {
                0 => "false",
                255 => "null",
                _ => "true",
            });
        } else if self.known_type == 5 {
            let text_nr = self.store().get_u32_raw(self.rec, self.pos);
            if text_nr == 0 || text_nr >= self.store().capacity_words() {
                if self.json || self.loft {
                    // Null text renders as `null` in JSON and in loft (loft has a
                    // `null` literal) — a re-parseable absence, not the `<bad-text>`
                    // debug tag.
                    s.push_str("null");
                } else {
                    write!(s, "<bad-text:{text_nr}>").unwrap();
                }
            } else {
                let text_val = self.store().get_str(text_nr);
                if self.json || self.loft {
                    // loft string literals accept the same escapes as JSON
                    // (`\"`, `\n`, `\\`, …), so the JSON escaper produces a
                    // re-parseable loft text literal too.
                    write_json_escaped(s, text_val);
                } else {
                    s.push('\"');
                    s.push_str(text_val);
                    s.push('\"');
                }
            }
        } else if self.known_type == 6 {
            let i = self.store().get_u32_raw(self.rec, self.pos);
            if i != u32::MAX
                && let Some(ch) = char::from_u32(i)
            {
                write!(s, "'{ch}'").unwrap();
            }
        } else if (self.known_type as usize) < self.stores.types.len() {
            match &self.stores.types[self.known_type as usize].parts {
                Parts::Enum(vals) => {
                    // P54 Q3 second half — when serialising in JSON mode
                    // and the parent enum is JsonValue, render the
                    // *value* of the variant (the JSON the JsonValue
                    // semantically represents), not the
                    // discriminant-tagged debug shape.  E.g. an inline
                    // JString { value: "hi" } renders as `"hi"`, not as
                    // `JString { value: "hi" }` or `{"JString":...}`.
                    if self.json && self.stores.types[self.known_type as usize].name == "JsonValue"
                    {
                        self.write_jsonvalue(s, indent);
                        return;
                    }
                    // @PLN25 single-payload — a synthetic `__nullable<S>` enum formats
                    // TRANSPARENTLY: a present element renders as the dense `S` (no `Some`
                    // wrapper), an absent one as `null`.  The dense `S` lives in the `Some`
                    // variant's inline `payload` field, so render a sub-view at the payload
                    // offset typed as the dense struct — its own `write` emits the loft type
                    // name / `{…}` body.  Without this, `{v[i]}` showed `{payload: {…}}`.
                    if self.stores.types[self.known_type as usize]
                        .name
                        .starts_with("__nullable<")
                    {
                        let v = self.store().get_byte(self.rec, self.pos, 0);
                        if v <= 0 {
                            s.push_str("null");
                        } else if (v as usize - 1) < vals.len() {
                            let some_tp = vals[v as usize - 1].0;
                            if some_tp != u16::MAX
                                && let Parts::EnumValue(_, st) =
                                    &self.stores.types[some_tp as usize].parts
                                && let Some(pf) = st.iter().find(|f| f.name == "payload")
                            {
                                let sub = ShowDb {
                                    stores: self.stores,
                                    store: self.store,
                                    rec: self.rec,
                                    pos: self.pos + u32::from(pf.position),
                                    known_type: pf.content,
                                    pretty: self.pretty,
                                    json: self.json,
                                    loft: self.loft,
                                    dump: self.dump,
                                    compact: self.compact,
                                    max_depth: self.max_depth,
                                    max_elements: self.max_elements,
                                };
                                sub.write(s, indent);
                            }
                        }
                        return;
                    }
                    let v = self.store().get_byte(self.rec, self.pos, 0);
                    let enum_val = if v <= 0 {
                        "null"
                    } else if (v as usize - 1) < vals.len() {
                        &vals[v as usize - 1].1
                    } else {
                        "?"
                    };
                    // loft qualifies a real variant as `Enum.Variant` so it
                    // re-parses unambiguously (a bare `Variant` can't infer its
                    // enum type in the language parser).
                    if self.loft && v > 0 && (v as usize - 1) < vals.len() {
                        write!(s, "{}.", self.stores.types[self.known_type as usize].name).unwrap();
                    }
                    s.push_str(enum_val);
                    let tp_nr = if v <= 0 || (v as usize - 1) >= vals.len() {
                        u16::MAX
                    } else {
                        vals[v as usize - 1].0
                    };
                    if tp_nr != u16::MAX
                        && let Parts::EnumValue(_, st) = &self.stores.types[tp_nr as usize].parts
                    {
                        s.push(' ');
                        self.write_struct(s, st, indent);
                    }
                }
                Parts::Struct(st) => {
                    // loft prefixes the type name → `TypeName{…}`, a re-parseable
                    // constructor; debug/JSON emit a bare object.
                    if self.loft {
                        s.push_str(&self.stores.types[self.known_type as usize].name);
                    }
                    self.write_struct(s, st, indent);
                }
                Parts::EnumValue(_, st) => {
                    // wrap struct-enum variant in a discriminant object so JSON
                    // round-trip can identify the variant: {"VariantName":{fields}}.
                    if self.json {
                        let variant_name = &self.stores.types[self.known_type as usize].name;
                        write!(s, "{{\"{variant_name}\":").unwrap();
                        self.write_struct(s, st, indent);
                        s.push('}');
                    } else {
                        // loft emits the variant as its own constructor `V{…}`.
                        if self.loft {
                            s.push_str(&self.stores.types[self.known_type as usize].name);
                        }
                        self.write_struct(s, st, indent);
                    }
                }
                Parts::Vector(tp)
                | Parts::Sorted(tp, _)
                | Parts::Array(tp)
                | Parts::Ordered(tp, _)
                | Parts::Hash(tp, _)
                | Parts::Index(tp, _, _)
                | Parts::Spacial(tp, _) => {
                    self.write_list(s, *tp, indent);
                }
                Parts::Byte(from, nullable) => {
                    let v = self.store().get_byte(self.rec, self.pos, *from);
                    if *nullable && v == 255 {
                        s.push_str("null");
                    } else {
                        write!(s, "{v}").unwrap();
                    }
                }
                Parts::Short(from, nullable) => {
                    let v = self.store().get_short(self.rec, self.pos, *from);
                    if *nullable && v == 65535 {
                        s.push_str("null");
                    } else {
                        write!(s, "{v}").unwrap();
                    }
                }
                Parts::ShortRaw(from, nullable) => {
                    let v = self.store().get_i16_raw(self.rec, self.pos, *from);
                    if *nullable && v == i32::MIN {
                        s.push_str("null");
                    } else {
                        write!(s, "{v}").unwrap();
                    }
                }
                Parts::Int(_, nullable) => {
                    let v = self.store().get_i32_raw(self.rec, self.pos);
                    if *nullable && v == i32::MIN {
                        s.push_str("null");
                    } else {
                        write!(s, "{v}").unwrap();
                    }
                }
                // Plan-06 phase 4d.C step 2: format a stored DbRef
                // pointer as the three u32 components.  Closures don't
                // round-trip through textual format anyway; this just
                // gives a recognisable shape for debug output.
                Parts::DbRef => {
                    let store = self.store();
                    let store_nr = store.get_u32_raw(self.rec, self.pos) as u16;
                    let rec = store.get_u32_raw(self.rec, self.pos + 4);
                    let pos = store.get_u32_raw(self.rec, self.pos + 8);
                    if store_nr == u16::MAX && rec == 0 {
                        s.push_str("null");
                    } else {
                        write!(s, "DbRef({store_nr},{rec},{pos})").unwrap();
                    }
                }
                // P213: format a child-record rec-id pointer.  Closures
                // don't round-trip through textual format; debug shape only.
                Parts::ChildRec(_) => {
                    let rec = self.store().get_u32_raw(self.rec, self.pos);
                    if rec == 0 {
                        s.push_str("null");
                    } else {
                        write!(s, "ChildRec({rec})").unwrap();
                    }
                }
                Parts::Base => {
                    panic!(
                        "Not matching parts:{:?} type:{} name:{}",
                        self.stores.types[self.known_type as usize].parts,
                        self.known_type,
                        self.stores.types[self.known_type as usize].name
                    )
                }
            }
        } else {
            panic!("Undefined known type {}", self.known_type)
        }
    }

    fn write_indent(&self, complex: bool, s: &mut String, indent: u16, zero_test: bool) {
        if complex && zero_test {
            s.push_str(&ShowDb::new_line(indent + 1));
        } else if self.pretty {
            s.push(' ');
        }
    }

    fn write_struct(&self, s: &mut String, fields: &[Field], indent: u16) {
        // Bounded render (the debugger's variables glance): stop descending past the depth
        // limit.  In loft mode the type name is already emitted, so this reads `TypeName{...}`.
        // Never fires for the round-tripping serializers (they pass `max_depth == u16::MAX`),
        // so `show_loft` / `to_json` output stays byte-identical.
        if indent >= self.max_depth {
            s.push_str("{...}");
            return;
        }
        // P54 Q3 second half — when serialising in JSON pretty mode,
        // ALWAYS use the multi-line shape regardless of the type's
        // `complex` flag.  `complex` is set on collection types
        // (vectors, hashes, etc.) for debug-display purposes; it
        // doesn't reflect whether the JSON output should be pretty
        // (newlines + indent vs. inline spaces).  For canonical JSON
        // pretty output, every non-empty struct should multi-line.
        let complex =
            self.pretty && (self.json || self.stores.types[self.known_type as usize].complex);
        let any_visible = self.has_visible_field(fields);
        // TODO reference to an object inside a field instead of the object itself, show the key
        s.push('{');
        // JSON pretty mode opens the body with a newline + indent
        // (when there's something to emit).  Debug-pretty mode opens
        // with a single space (compact `{ a: 1 }` shape) to match
        // the existing dump format.
        if complex && self.json && any_visible {
            s.push_str(&ShowDb::new_line(indent + 1));
        } else if self.pretty {
            s.push(' ');
        }
        self.write_fields(s, fields, indent, complex);
        if complex && any_visible {
            s.push_str(&ShowDb::new_line(indent));
        } else if self.pretty && !complex {
            s.push(' ');
        }
        s.push('}');
    }

    /// Return true iff `fields` contains at least one entry that
    /// `write_fields` would emit (skips internal `#`-prefixed names,
    /// the `enum` discriminator, and null-valued slots).  Used by
    /// `write_struct` so the JSON-pretty open-brace newline only
    /// fires when there's something inside.
    fn has_visible_field(&self, fields: &[Field]) -> bool {
        for fld in fields {
            if fld.name == "enum" {
                continue;
            }
            if fld.name.starts_with('#')
                || (!fld.other_indexes.is_empty() && fld.other_indexes[0] == u16::MAX)
                || self.stores.is_null(
                    self.store(),
                    self.rec,
                    self.pos + u32::from(fld.position),
                    fld.content,
                )
            {
                continue;
            }
            return true;
        }
        false
    }

    fn write_fields(&self, s: &mut String, fields: &[Field], indent: u16, complex: bool) {
        let mut first = true;
        for fld in fields {
            if fld.name == "enum" {
                continue;
            }
            if fld.name.starts_with('#')
                || (!fld.other_indexes.is_empty() && fld.other_indexes[0] == u16::MAX)
                || self.stores.is_null(
                    self.store(),
                    self.rec,
                    self.pos + u32::from(fld.position),
                    fld.content,
                )
            {
                continue;
            }
            if first {
                first = false;
            } else {
                s.push(',');
                self.write_indent(complex, s, indent, true);
            }
            if self.json {
                s.push('"');
            }
            s.push_str(&fld.name);
            if self.json {
                s.push('"');
            }
            s.push(':');
            if self.pretty {
                s.push(' ');
            }
            let sub = ShowDb {
                stores: self.stores,
                store: self.store,
                rec: self.rec,
                pos: self.pos + u32::from(fld.position),
                known_type: fld.content,
                pretty: self.pretty,
                json: self.json,
                loft: self.loft,
                dump: self.dump,
                compact: self.compact,
                max_depth: self.max_depth,
                max_elements: self.max_elements,
            };
            sub.write(s, indent + 1);
        }
    }

    fn new_line(indent: u16) -> String {
        let mut res = "\n".to_string();
        for _ in 0..indent {
            res += "  ";
        }
        res
    }

    fn write_list(&self, s: &mut String, content: u16, indent: u16) {
        let data = DbRef {
            store_nr: self.store,
            rec: self.rec,
            pos: self.pos,
        };
        // Bounded render: a vector past the depth limit collapses to `[...]` (debugger
        // glance).  Unlimited (`max_depth == u16::MAX`) never triggers, so round-trip output
        // is unchanged.
        if indent >= self.max_depth {
            s.push_str("[...]");
            return;
        }
        let complex = self.pretty && self.stores.types[content as usize].complex;
        s.push('[');
        if matches!(
            self.stores.types[self.known_type as usize].parts,
            Parts::Hash(_, _)
        ) {
            self.write_hash(s, content, indent, &data, complex);
            return;
        }
        let mut pos = i32::MAX;
        let mut first_elm = true;
        let mut count: u32 = 0;
        loop {
            if data.rec == 0 {
                break;
            }
            let rec = self.stores.next(&data, &mut pos, self.known_type);
            if rec.rec == 0 {
                break;
            }
            // Bounded render: stop after `max_elements` and mark the tail.  `count` is `u32`
            // (never wraps) and the guard is gated on a finite limit, so the unlimited
            // serializers emit every element exactly as before.
            if self.max_elements != u16::MAX && count >= u32::from(self.max_elements) {
                s.push_str(",...");
                break;
            }
            if first_elm {
                if self.pretty {
                    self.write_indent(complex, s, indent, true);
                }
                first_elm = false;
            } else {
                s.push(',');
                if self.pretty {
                    if matches!(
                        self.stores.types[content as usize].parts,
                        Parts::Struct(_) | Parts::EnumValue(_, _)
                    ) {
                        self.write_indent(true, s, indent, true);
                    } else {
                        self.write_indent(complex, s, indent, false);
                    }
                }
            }
            let sub = ShowDb {
                stores: self.stores,
                store: self.store,
                rec: rec.rec,
                pos: rec.pos,
                known_type: content,
                pretty: self.pretty,
                json: self.json,
                loft: self.loft,
                dump: self.dump,
                compact: self.compact,
                max_depth: self.max_depth,
                max_elements: self.max_elements,
            };
            sub.write(s, indent + 1);
            count += 1;
        }
        if self.pretty {
            s.push(' ');
        }
        s.push(']');
    }

    #[allow(dead_code)]
    fn write_hash(&self, s: &mut String, content: u16, indent: u16, data: &DbRef, complex: bool) {
        let mut map = BTreeMap::new();
        let mut pos = i32::MAX;
        let rec = self
            .stores
            .store_nr(self.store)
            .get_u32_raw(data.rec, data.pos);
        if rec == 0 {
            s.push(']');
            return;
        }
        let max_pos = *self.stores.store_nr(self.store).addr::<i32>(rec, 0) * 8;
        loop {
            if pos == i32::MAX {
                pos = 8;
            } else if pos < max_pos - 4 {
                pos += 4;
            } else {
                break;
            }
            let rec = self
                .stores
                .store_nr(self.store)
                .get_i32_raw(rec, pos as u32);
            if rec != 0 {
                let r = DbRef {
                    store_nr: data.store_nr,
                    rec: rec as u32,
                    pos: 8,
                };
                let key = keys::get_simple(
                    &r,
                    &self.stores.allocations,
                    self.stores.keys(self.known_type),
                );
                map.insert(key, rec);
            }
        }
        let mut first_elm = true;
        for (_, p) in map {
            if first_elm {
                if self.pretty {
                    self.write_indent(complex, s, indent, true);
                }
                first_elm = false;
            } else {
                s.push(',');
                if self.pretty {
                    if matches!(self.stores.types[content as usize].parts, Parts::Struct(_)) {
                        self.write_indent(true, s, indent, true);
                    } else {
                        self.write_indent(complex, s, indent, false);
                    }
                }
            }
            let sub = ShowDb {
                stores: self.stores,
                store: self.store,
                rec: p as u32,
                pos: 8,
                known_type: content,
                pretty: self.pretty,
                json: self.json,
                loft: self.loft,
                dump: self.dump,
                compact: self.compact,
                max_depth: self.max_depth,
                max_elements: self.max_elements,
            };
            sub.write(s, indent + 1);
        }
        if self.pretty {
            s.push(' ');
        }
        s.push(']');
    }

    /// Render a JsonValue inline subtree as canonical RFC 8259 JSON.
    /// `self.rec` / `self.pos` point at the JsonValue's discriminant
    /// byte; the variant payload immediately follows at offsets given
    /// by `position(<variant_tp>, <field>)`.  Mirrors the dispatch
    /// in `src/native.rs::json_to_text_at` so a JsonValue field
    /// inside a struct round-trips identically whether it was
    /// rendered standalone via `json_value.to_json()` or as part of
    /// the parent struct via `parent.to_json()`.
    fn write_jsonvalue(&self, s: &mut String, indent: u16) {
        const JV_NULL: i32 = 1;
        const JV_BOOL: i32 = 2;
        const JV_NUMBER: i32 = 3;
        const JV_STRING: i32 = 4;
        const JV_ARRAY: i32 = 5;
        const JV_OBJECT: i32 = 6;
        let store = self.store();
        let discr = store.get_byte(self.rec, self.pos, 0);
        match discr {
            JV_NULL => s.push_str("null"),
            JV_BOOL => {
                let bool_tp = self.stores.name("JBool");
                let val_pos = u32::from(self.stores.position(bool_tp, "value")) + self.pos;
                let b = store.get_byte(self.rec, val_pos, 0);
                s.push_str(if b != 0 { "true" } else { "false" });
            }
            JV_NUMBER => {
                let num_tp = self.stores.name("JNumber");
                let val_pos = u32::from(self.stores.position(num_tp, "value")) + self.pos;
                let n = store.get_float(self.rec, val_pos);
                if n.is_finite() {
                    write!(s, "{n}").unwrap();
                } else {
                    s.push_str("null");
                }
            }
            JV_STRING => {
                let str_tp = self.stores.name("JString");
                let val_pos = u32::from(self.stores.position(str_tp, "value")) + self.pos;
                let s_rec = store.get_u32_raw(self.rec, val_pos);
                if s_rec == 0 {
                    s.push_str("null");
                } else {
                    let raw = store.get_str(s_rec).to_string();
                    write_json_escaped(s, &raw);
                }
            }
            JV_ARRAY => {
                let array_tp = self.stores.name("JArray");
                let items_pos = u32::from(self.stores.position(array_tp, "items")) + self.pos;
                let items_rec = store.get_i32_raw(self.rec, items_pos);
                if items_rec <= 0 {
                    s.push_str("[]");
                    return;
                }
                let length = i64::from(store.get_u32_raw(items_rec as u32, 4));
                if length <= 0 {
                    s.push_str("[]");
                    return;
                }
                let jv_tp = self.stores.name("JsonValue");
                let jv_size = u32::from(self.stores.size(jv_tp));
                s.push('[');
                for i in 0..length {
                    if i > 0 {
                        s.push(',');
                    }
                    if self.pretty {
                        s.push('\n');
                        for _ in 0..=indent {
                            s.push_str("  ");
                        }
                    }
                    let elm_offset = 8u32 + u32::try_from(i).expect("non-negative") * jv_size;
                    let sub = ShowDb {
                        stores: self.stores,
                        store: self.store,
                        rec: items_rec as u32,
                        pos: elm_offset,
                        known_type: jv_tp,
                        pretty: self.pretty,
                        json: true,
                        loft: false,
                        dump: self.dump,
                        compact: self.compact,
                        max_depth: self.max_depth,
                        max_elements: self.max_elements,
                    };
                    sub.write_jsonvalue(s, indent + 1);
                }
                if self.pretty {
                    s.push('\n');
                    for _ in 0..indent {
                        s.push_str("  ");
                    }
                }
                s.push(']');
            }
            JV_OBJECT => {
                let obj_tp = self.stores.name("JObject");
                let fields_pos = u32::from(self.stores.position(obj_tp, "fields")) + self.pos;
                let fields_rec = store.get_i32_raw(self.rec, fields_pos);
                if fields_rec <= 0 {
                    s.push_str("{}");
                    return;
                }
                let length = i64::from(store.get_u32_raw(fields_rec as u32, 4));
                if length <= 0 {
                    s.push_str("{}");
                    return;
                }
                let jfield_tp = self.stores.name("JsonField");
                let jf_size = u32::from(self.stores.size(jfield_tp));
                let name_field_pos = u32::from(self.stores.position(jfield_tp, "name"));
                let value_field_pos = u32::from(self.stores.position(jfield_tp, "value"));
                let jv_tp = self.stores.name("JsonValue");
                s.push('{');
                for i in 0..length {
                    if i > 0 {
                        s.push(',');
                    }
                    if self.pretty {
                        s.push('\n');
                        for _ in 0..=indent {
                            s.push_str("  ");
                        }
                    }
                    let elm_offset = 8u32 + u32::try_from(i).expect("non-negative") * jf_size;
                    let name_rec =
                        store.get_u32_raw(fields_rec as u32, elm_offset + name_field_pos);
                    let raw = store.get_str(name_rec).to_string();
                    write_json_escaped(s, &raw);
                    s.push(':');
                    if self.pretty {
                        s.push(' ');
                    }
                    let sub = ShowDb {
                        stores: self.stores,
                        store: self.store,
                        rec: fields_rec as u32,
                        pos: elm_offset + value_field_pos,
                        known_type: jv_tp,
                        pretty: self.pretty,
                        json: true,
                        loft: false,
                        dump: self.dump,
                        compact: self.compact,
                        max_depth: self.max_depth,
                        max_elements: self.max_elements,
                    };
                    sub.write_jsonvalue(s, indent + 1);
                }
                if self.pretty {
                    s.push('\n');
                    for _ in 0..indent {
                        s.push_str("  ");
                    }
                }
                s.push('}');
            }
            _ => s.push_str("null"),
        }
    }
}

/// JSON-escape a string and wrap with `"`.  Escapes `"`, `\`, and
/// control characters per RFC 8259.  Used by `ShowDb` (json: true)
/// so `T.to_json()` produces canonical JSON for any text field
/// containing quotes, backslashes, or control bytes.
/// Ensure a default-formatted float carries a decimal point or exponent, so the
/// loft serializer emits `3.0` not `3` — the latter would re-parse as an
/// integer and mismatch a `float`/`single` field.  Leaves `3.14`, `1e10`, and
/// the non-numeric `inf`/`NaN` forms untouched.
fn ensure_decimal(formatted: &str) -> String {
    let has_dot_or_exp = formatted
        .bytes()
        .any(|b| b == b'.' || b == b'e' || b == b'E');
    let has_digit = formatted.bytes().any(|b| b.is_ascii_digit());
    if has_dot_or_exp || !has_digit {
        formatted.to_string()
    } else {
        format!("{formatted}.0")
    }
}

pub(crate) fn write_json_escaped(out: &mut String, raw: &str) {
    out.push('"');
    for ch in raw.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                write!(out, "\\u{:04x}", c as u32).unwrap();
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

// ─── DumpDb: structured debug dump with references and limits ────────────────

impl Stores {
    /// Produce a structured debug dump string showing store/record references.
    /// Multi-line with indentation when `compact` is false.
    #[must_use]
    pub fn dump_data(&self, db: &DbRef, tp: u16, max_depth: u16, max_elements: u16) -> String {
        let mut s = String::new();
        ShowDb {
            stores: self,
            store: db.store_nr,
            rec: db.rec,
            pos: db.pos,
            known_type: tp,
            pretty: false,
            json: false,
            loft: false,
            dump: true,
            compact: false,
            max_depth,
            max_elements,
        }
        .write(&mut s, 0);
        s
    }

    /// Compact single-line dump for inline trace output.
    #[must_use]
    pub fn dump_compact(&self, db: &DbRef, tp: u16, max_depth: u16, max_elements: u16) -> String {
        let mut s = String::new();
        ShowDb {
            stores: self,
            store: db.store_nr,
            rec: db.rec,
            pos: db.pos,
            known_type: tp,
            pretty: false,
            json: false,
            loft: false,
            dump: true,
            compact: true,
            max_depth,
            max_elements,
        }
        .write(&mut s, 0);
        s
    }
}

impl ShowDb<'_> {
    /// A child walker carrying *this* walker's mode + limits — the one place the dump
    /// recursion constructs a sub-record (so every field/element inherits `dump`, `compact`,
    /// and the depth/element limits).
    fn dump_child(&self, rec: u32, pos: u32, known_type: u16) -> ShowDb<'_> {
        ShowDb {
            stores: self.stores,
            store: self.store,
            rec,
            pos,
            known_type,
            pretty: false,
            json: false,
            loft: false,
            dump: true,
            compact: self.compact,
            max_depth: self.max_depth,
            max_elements: self.max_elements,
        }
    }

    fn dump_sep(&self, s: &mut String, level: u16) {
        if self.compact {
            s.push(' ');
        } else {
            s.push('\n');
            for _ in 0..level {
                s.push_str("  ");
            }
        }
    }

    /// The **dump** mode (the old `DumpDb`): the structured trace form with `#store.rec`
    /// references and depth/element truncation.  `indent` doubles as the nesting depth.
    pub fn write_dump(&self, s: &mut String, indent: u16) {
        if self.rec == 0 {
            s.push_str("null");
            return;
        }
        // Guard: ensure the record is within the store's buffer before reading.
        let store = self.store();
        if u64::from(self.rec) * 8 + u64::from(self.pos) + 8 > store.byte_capacity() {
            write!(s, "<oob:rec={},pos={}>", self.rec, self.pos).unwrap();
            return;
        }
        match self.known_type {
            0 => write!(s, "{}", self.store().get_int(self.rec, self.pos)).unwrap(), // integer
            1 => write!(s, "{}l", self.store().get_long(self.rec, self.pos)).unwrap(), // long
            2 => write!(s, "{}f", self.store().get_single(self.rec, self.pos)).unwrap(), // single
            3 => write!(s, "{}", self.store().get_float(self.rec, self.pos)).unwrap(), // float
            // 255 = @PLN17's three-state-boolean null sentinel (C73); inert for two-state.
            4 => s.push_str(match self.store().get_byte(self.rec, self.pos, 0) {
                0 => "false",
                255 => "null",
                _ => "true",
            }),
            5 => {
                // text
                let text_nr = self.store().get_u32_raw(self.rec, self.pos);
                if text_nr == 0 || text_nr >= self.store().capacity_words() {
                    write!(s, "<bad-text:{text_nr}>").unwrap();
                } else {
                    let text_val = self.store().get_str(text_nr);
                    write!(s, "\"{}\"", text_val.replace('"', "\\\"")).unwrap();
                }
            }
            6 => {
                // character
                let i = self.store().get_u32_raw(self.rec, self.pos);
                if let Some(ch) = char::from_u32(i) {
                    write!(s, "'{ch}'").unwrap();
                } else {
                    write!(s, "'?{i}'").unwrap();
                }
            }
            tp if (tp as usize) < self.stores.types.len() => {
                self.write_dump_typed(s, indent);
            }
            tp => write!(s, "?type({tp})").unwrap(),
        }
    }

    fn write_dump_typed(&self, s: &mut String, indent: u16) {
        match &self.stores.types[self.known_type as usize].parts.clone() {
            Parts::Enum(vals) => {
                let v = self.store().get_byte(self.rec, self.pos, 0);
                let name = if v <= 0 {
                    "null"
                } else if (v as usize - 1) < vals.len() {
                    &vals[v as usize - 1].1
                } else {
                    "?"
                };
                s.push_str(name);
                let tp_nr = if v <= 0 || (v as usize - 1) >= vals.len() {
                    u16::MAX
                } else {
                    vals[v as usize - 1].0
                };
                if tp_nr != u16::MAX
                    && let Parts::EnumValue(_, st) = &self.stores.types[tp_nr as usize].parts
                {
                    s.push(' ');
                    self.write_dump_struct(s, st, indent);
                }
            }
            Parts::Struct(st) | Parts::EnumValue(_, st) => {
                self.write_dump_struct(s, st, indent);
            }
            Parts::Vector(tp)
            | Parts::Sorted(tp, _)
            | Parts::Array(tp)
            | Parts::Ordered(tp, _)
            | Parts::Index(tp, _, _) => {
                self.write_dump_list(s, *tp, indent);
            }
            Parts::Hash(_, _) | Parts::Spacial(_, _) => {
                // Hash and Spacial don't support sequential next() — show count only.
                let data = DbRef {
                    store_nr: self.store,
                    rec: self.rec,
                    pos: self.pos,
                };
                let len = vector::length_vector(&data, &self.stores.allocations);
                write!(s, "#{}.? [{len} items]", self.store).unwrap();
            }
            Parts::Byte(from, nullable) => {
                let v = self.store().get_byte(self.rec, self.pos, *from);
                if *nullable && v == 255 {
                    s.push_str("null");
                } else {
                    write!(s, "{v}").unwrap();
                }
            }
            Parts::Short(from, nullable) => {
                let v = self.store().get_short(self.rec, self.pos, *from);
                if *nullable && v == 65535 {
                    s.push_str("null");
                } else {
                    write!(s, "{v}").unwrap();
                }
            }
            Parts::ShortRaw(from, nullable) => {
                let v = self.store().get_i16_raw(self.rec, self.pos, *from);
                if *nullable && v == i32::MIN {
                    s.push_str("null");
                } else {
                    write!(s, "{v}").unwrap();
                }
            }
            Parts::Int(_, nullable) => {
                let v = self.store().get_i32_raw(self.rec, self.pos);
                if *nullable && v == i32::MIN {
                    s.push_str("null");
                } else {
                    write!(s, "{v}").unwrap();
                }
            }
            Parts::DbRef => {
                let store_nr = self.store().get_u32_raw(self.rec, self.pos);
                let rec = self.store().get_u32_raw(self.rec, self.pos + 4);
                let pos = self.store().get_u32_raw(self.rec, self.pos + 8);
                if rec == 0 {
                    s.push_str("null");
                } else {
                    write!(s, "DbRef({store_nr},{rec},{pos})").unwrap();
                }
            }
            // P213: format a child-record rec-id pointer.
            Parts::ChildRec(_) => {
                let rec = self.store().get_u32_raw(self.rec, self.pos);
                if rec == 0 {
                    s.push_str("null");
                } else {
                    write!(s, "ChildRec({rec})").unwrap();
                }
            }
            Parts::Base => {
                write!(s, "?base({})", self.known_type).unwrap();
            }
        }
    }

    fn write_dump_struct(&self, s: &mut String, fields: &[Field], indent: u16) {
        // Show store:record reference
        write!(s, "#{}.{}", self.store, self.rec).unwrap();
        if indent >= self.max_depth {
            s.push_str(" {...}");
            return;
        }
        s.push_str(" {");
        let mut first = true;
        for fld in fields {
            if fld.name == "enum" || fld.name.starts_with('#') {
                continue;
            }
            if self.stores.is_null(
                self.store(),
                self.rec,
                self.pos + u32::from(fld.position),
                fld.content,
            ) {
                continue;
            }
            if !first {
                s.push(',');
            }
            first = false;
            self.dump_sep(s, indent + 1);
            s.push_str(&fld.name);
            s.push_str(": ");
            self.dump_child(self.rec, self.pos + u32::from(fld.position), fld.content)
                .write_dump(s, indent + 1);
        }
        self.dump_sep(s, indent);
        s.push('}');
    }

    fn write_dump_list(&self, s: &mut String, content: u16, indent: u16) {
        let data = DbRef {
            store_nr: self.store,
            rec: self.rec,
            pos: self.pos,
        };
        // Show the vector record reference
        let vec_rec = if data.rec > 0 {
            self.store().get_u32_raw(data.rec, data.pos)
        } else {
            0
        };
        write!(s, "#{}.{}", self.store, vec_rec).unwrap();
        if indent >= self.max_depth {
            let len = vector::length_vector(&data, &self.stores.allocations);
            write!(s, " [{len} items...]").unwrap();
            return;
        }
        s.push_str(" [");
        let mut pos = i32::MAX;
        let mut count: u16 = 0;
        loop {
            if data.rec == 0 {
                break;
            }
            let rec = self.stores.next(&data, &mut pos, self.known_type);
            if rec.rec == 0 {
                break;
            }
            if count >= self.max_elements {
                self.dump_sep(s, indent + 1);
                let remaining =
                    vector::length_vector(&data, &self.stores.allocations) as u16 - count;
                write!(s, "...{remaining} more").unwrap();
                break;
            }
            if count > 0 {
                s.push(',');
            }
            self.dump_sep(s, indent + 1);
            self.dump_child(rec.rec, rec.pos, content)
                .write_dump(s, indent + 1);
            count += 1;
        }
        self.dump_sep(s, indent);
        s.push(']');
    }
}

#[cfg(test)]
mod json_escape_tests {
    //! Unit-level coverage for `write_json_escaped` — backs the JSON
    //! string-emitting paths in `Stores::show_json` (P54 Q3 second
    //! half: `T.to_json()`).  Higher-level coverage exercising the
    //! loft method-call surface lives in
    //! `tests/issues.rs::q3b_struct_to_json_*`.
    use super::write_json_escaped;

    fn esc(s: &str) -> String {
        let mut out = String::new();
        write_json_escaped(&mut out, s);
        out
    }

    #[test]
    fn empty_string_renders_as_quoted_empty() {
        assert_eq!(esc(""), r#""""#);
    }

    #[test]
    fn ascii_passes_through_unchanged() {
        assert_eq!(esc("hello world"), r#""hello world""#);
    }

    #[test]
    fn double_quote_is_backslash_escaped() {
        assert_eq!(esc(r#"a"b"#), r#""a\"b""#);
    }

    #[test]
    fn backslash_is_doubled() {
        assert_eq!(esc(r"a\b"), r#""a\\b""#);
    }

    #[test]
    fn newline_uses_short_form() {
        assert_eq!(esc("a\nb"), r#""a\nb""#);
    }

    #[test]
    fn tab_uses_short_form() {
        assert_eq!(esc("a\tb"), r#""a\tb""#);
    }

    #[test]
    fn carriage_return_uses_short_form() {
        assert_eq!(esc("a\rb"), r#""a\rb""#);
    }

    #[test]
    fn backspace_uses_short_form() {
        assert_eq!(esc("a\x08b"), r#""a\bb""#);
    }

    #[test]
    fn form_feed_uses_short_form() {
        assert_eq!(esc("a\x0cb"), r#""a\fb""#);
    }

    #[test]
    fn other_low_control_chars_use_unicode_escape() {
        // 0x01 hits the catch-all `(c as u32) < 0x20` arm —
        // emits canonical `\uXXXX`, not a literal control byte.
        assert_eq!(esc("a\x01b"), "\"a\\u0001b\"");
        // 0x1f is the highest control char that takes the unicode form.
        assert_eq!(esc("\x1f"), "\"\\u001f\"");
    }

    #[test]
    fn space_passes_through_as_first_non_control_char() {
        // 0x20 is space — boundary case for the `< 0x20` test.
        assert_eq!(esc(" "), r#"" ""#);
    }

    #[test]
    fn utf8_multibyte_passes_through_unchanged() {
        // RFC 8259 allows literal UTF-8 bytes; we don't `\u`-escape
        // them.  The Rust crab emoji and accented chars round-trip
        // as their literal UTF-8 bytes.
        assert_eq!(esc("🦀"), r#""🦀""#);
        assert_eq!(esc("café"), r#""café""#);
    }

    #[test]
    fn quotes_inside_unicode_string_still_escape() {
        // Mixed ASCII + UTF-8 + quote — proves the per-char dispatch
        // doesn't fall back to "everything passes through" for
        // non-ASCII strings.
        assert_eq!(esc(r#"x"é"y"#), r#""x\"é\"y""#);
    }
}
