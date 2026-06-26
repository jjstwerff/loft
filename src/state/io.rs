// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use super::{State, new_ref, size_ref};
use crate::database::{Parts, ShowDb};
use crate::keys::{Content, DbRef, Key};
use crate::{hash, tree, vector};
#[cfg(not(feature = "wasm"))]
use std::fs::{File, OpenOptions};
#[cfg(not(feature = "wasm"))]
use std::io::{Read, Seek, SeekFrom, Write};

impl State {
    /**
    Read data from a file
    # Panics
    When the reading was incorrect.
    */
    pub fn get_file_text(&mut self) {
        let r = *self.get_stack::<DbRef>();
        let file = *self.get_stack::<DbRef>();
        if file.rec == 0 {
            return;
        }
        #[cfg(feature = "wasm")]
        {
            // FS-B: read entire file via JS host bridge.
            let file_path = {
                let store = self.database.store(&file);
                store
                    .get_str(store.get_u32_raw(file.rec, file.pos + 24))
                    .to_owned()
            };
            let buf = self.database.store_mut(&r).addr_mut::<String>(r.rec, r.pos);
            // @P349: consult VIRT_FS first — `compile_and_run` populates it with
            // every file passed to the playground/gallery (the program's bundled
            // data).  Without this, runtime `file("x").content()` only hit the JS
            // `fs_read_text` host bridge, which the browser playground does not
            // provide → empty string (the lexer already reads source/lib files
            // from VIRT_FS via `virt_fs_get`, but the runtime read did not).  A
            // real host with no VIRT_FS entry for the path still falls through to
            // its `host_fs_read_text` bridge, so live-FS hosts are unaffected.
            if let Some(text) = crate::wasm::virt_fs_get(&file_path) {
                *buf = text;
            } else if let Some(text) = crate::wasm::host_fs_read_text(&file_path) {
                *buf = text;
            }
        }
        // QUALITY Tier 3 #9: explicit stub for `wasm32-unknown-unknown` without
        // the `wasm` host-bridge feature (the `--html` browser build target).
        // `std::fs::File::open` compiles there but fails at runtime in a way
        // that depends on the embedding's panic-hook configuration.  The stub
        // always leaves `buf` untouched — same observable semantics as a file
        // that doesn't exist on native — so a `--html` program calling
        // `file("x").content()` reliably gets an empty String instead of
        // racing against an unpredictable runtime error.
        //
        // @P334 fix (2026-05-29): narrowed from `target_arch = "wasm32"` to
        // also exclude `target_os = "wasi"`, so wasip2 (which HAS a working
        // FS via WASI preopens) falls through to the std::fs impl.  Mirrors
        // the same fix at `src/database/io.rs::get_file`.
        #[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
        {
            let _ = (file, r);
        }
        #[cfg(all(
            not(feature = "wasm"),
            any(not(target_arch = "wasm32"), target_os = "wasi")
        ))]
        {
            let store = self.database.store(&file);
            let raw_path = store
                .get_str(store.get_u32_raw(file.rec, file.pos + 24))
                .to_owned();
            // #255 / @PLN9: re-home a relative path against the program anchor.
            let path_string = self.database.resolve_path(&raw_path);
            let buf = self.database.store_mut(&r).addr_mut::<String>(r.rec, r.pos);
            if let Ok(mut f) = File::open(&path_string) {
                match f.read_to_string(buf) {
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                        // non-UTF-8 bytes.  Silently clearing
                        // the buffer masks real data — write a loud
                        // actionable warning to stderr so the user
                        // sees the misuse the first time the call
                        // fires.  Return "" for backwards-compat
                        // (callers that guarded on "" as "could
                        // not read" still work).
                        buf.clear();
                        let size = std::fs::metadata(&path_string).map_or(0, |m| m.len());
                        eprintln!(
                            "warning: file({path_string:?}).content() got non-UTF-8 \
                             bytes ({size} bytes in file) — returning empty text. \
                             For binary files use `f#format = LittleEndian; f#read(n)` \
                             (or `BigEndian`); see the loft-write skill § File I/O."
                        );
                    }
                    Err(_) => buf.clear(),
                }
            }
        }
    }

    /// Assemble the raw bytes to write for `val` of type `db_tp`.
    /// Shared by the native and WASM write paths.
    fn assemble_write_data(&self, val: DbRef, db_tp: u16, little_endian: bool) -> Vec<u8> {
        let mut data = Vec::new();
        if self.database.is_text_type(db_tp) {
            let store = self.database.store(&val);
            let s: &String = store.addr::<String>(val.rec, val.pos);
            data.extend_from_slice(s.as_bytes());
        } else if matches!(
            &self.database.types[db_tp as usize].parts,
            Parts::Byte(_, _) | Parts::Short(_, _) | Parts::ShortRaw(_, _) | Parts::Int(_, _)
        ) {
            // Narrow-integer values sit in an 8B variable slot stored raw as i64
            // (OpPutInt), not via the +1-encoded Parts::Byte/Short encoding that
            // structs use (nor the i32::MIN null sentinel of Parts::Int).
            // Read the raw i64 and serialise directly.
            let v = self.database.store(&val).get_int(val.rec, val.pos);
            match &self.database.types[db_tp as usize].parts {
                Parts::Byte(_, _) => data.push(v as u8),
                Parts::Short(_, _) | Parts::ShortRaw(_, _) => {
                    let b = if little_endian {
                        (v as u16).to_le_bytes()
                    } else {
                        (v as u16).to_be_bytes()
                    };
                    data.extend_from_slice(&b);
                }
                Parts::Int(_, _) => {
                    let b = if little_endian {
                        (v as i32).to_le_bytes()
                    } else {
                        (v as i32).to_be_bytes()
                    };
                    data.extend_from_slice(&b);
                }
                _ => unreachable!(),
            }
        } else if let Parts::Vector(elem_tp) = &self.database.types[db_tp as usize].parts {
            let elem_tp = *elem_tp;
            let vec_ref = *self.database.store(&val).addr::<DbRef>(val.rec, val.pos);
            let (v_ptr, store_nr) = {
                let store = self.database.store(&vec_ref);
                (
                    store.get_u32_raw(vec_ref.rec, vec_ref.pos),
                    vec_ref.store_nr,
                )
            };
            if v_ptr != 0 {
                let length = self.database.allocations[store_nr as usize].get_u32_raw(v_ptr, 4);
                let elem_size = u32::from(self.database.size(elem_tp));
                for i in 0..length {
                    let elem = DbRef {
                        store_nr,
                        rec: v_ptr,
                        pos: 8 + elem_size * i,
                    };
                    self.database
                        .read_data(&elem, elem_tp, little_endian, &mut data);
                }
            }
        } else {
            self.database
                .read_data(&val, db_tp, little_endian, &mut data);
        }
        data
    }

    pub fn write_file(&mut self) {
        let val = *self.get_stack::<DbRef>();
        let file = *self.get_stack::<DbRef>();
        let db_tp = self.code::<u16>();
        if file.rec == 0 {
            return;
        }
        let format = self
            .database
            .store(&file)
            .get_byte(file.rec, file.pos + 32, 0);
        // format: 1=TextFile, 2=LittleEndian, 3=BigEndian, 5=NotExists (default to TextFile).
        if format != 1 && format != 5 && format != 2 && format != 3 {
            return;
        }
        let little_endian = format == 2;
        let raw_next = self.database.store(&file).get_long(file.rec, file.pos + 16);
        // `+=` is append-only: when the user has not explicitly set
        // `f#next = N`, the write position is the current end of file
        // so successive `f += …` calls append (consistent with
        // `vector += [elem]` / `text += "more"`).  An explicit
        // `f#next = N` still overwrites at offset N.  Call
        // `f.set_file_size(0)` before the first write to truncate.
        let next_pos = if raw_next == i64::MIN {
            #[cfg(feature = "wasm")]
            {
                let file_path = {
                    let store = self.database.store(&file);
                    store
                        .get_str(store.get_u32_raw(file.rec, file.pos + 24))
                        .to_owned()
                };
                let sz = crate::wasm::host_fs_file_size(&file_path);
                if sz < 0 { 0 } else { sz }
            }
            #[cfg(not(feature = "wasm"))]
            {
                let path = {
                    let store = self.database.store(&file);
                    store
                        .get_str(store.get_u32_raw(file.rec, file.pos + 24))
                        .to_owned()
                };
                // #255 / @PLN9: re-home against the program anchor.
                let path = self.database.resolve_path(&path);
                std::fs::metadata(&path).map_or(0, |m| m.len() as i64)
            }
        } else {
            raw_next
        };
        self.database
            .store_mut(&file)
            .set_long(file.rec, file.pos + 8, next_pos);
        let data = self.assemble_write_data(val, db_tp, little_endian);
        let written = data.len();
        #[cfg(feature = "wasm")]
        {
            // FS-C: seek to position then write; VirtFS writeBytes handles extension.
            let file_path = {
                let store = self.database.store(&file);
                store
                    .get_str(store.get_u32_raw(file.rec, file.pos + 24))
                    .to_owned()
            };
            crate::wasm::host_fs_seek(&file_path, next_pos);
            crate::wasm::host_fs_write_bytes(&file_path, &data);
        }
        #[cfg(not(feature = "wasm"))]
        {
            let f_nr = self.database.files.len() as i32;
            let file_ref = self
                .database
                .store(&file)
                .get_i32_raw(file.rec, file.pos + 28);
            let file_ref = if file_ref == i32::MIN {
                let file_name = {
                    let store = self.database.store(&file);
                    store
                        .get_str(store.get_u32_raw(file.rec, file.pos + 24))
                        .to_owned()
                };
                // #255 / @PLN9: re-home against the program anchor.
                let file_name = self.database.resolve_path(&file_name);
                // Open for read+write without truncating so that earlier
                // bytes are preserved.  Create the file if it does not
                // exist yet.  Explicit truncation happens via
                // `f.set_file_size(0)` (or `f#size = 0`).
                match OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(&file_name)
                {
                    Ok(mut f) => {
                        // Seek to the stored write position (end of file
                        // for default appends, explicit offset for
                        // `f#next = N`).
                        let _ = f.seek(SeekFrom::Start(next_pos as u64));
                        self.database
                            .store_mut(&file)
                            .set_i32_raw(file.rec, file.pos + 28, f_nr);
                        if format == 5 {
                            self.database
                                .store_mut(&file)
                                .set_byte(file.rec, file.pos + 32, 0, 1);
                        }
                        self.database.files.push(Some(f));
                        f_nr
                    }
                    Err(e) => {
                        eprintln!("file open error for {file_name:?}: {e}");
                        return;
                    }
                }
            } else {
                // Handle already open: sync OS handle with stored next_pos so
                // the user can `f#next = N` to overwrite bytes in place.
                if let Some(f) = &mut self.database.files[file_ref as usize] {
                    let _ = f.seek(SeekFrom::Start(next_pos as u64));
                }
                file_ref
            };
            if let Some(f) = &mut self.database.files[file_ref as usize]
                && let Err(e) = f.write_all(&data)
            {
                eprintln!("file write error: {e}");
            }
        }
        self.database
            .store_mut(&file)
            .set_long(file.rec, file.pos + 16, next_pos + written as i64);
    }

    /// Dispatch read bytes into `val` of type `db_tp`.
    /// Shared by the native and WASM read paths.
    fn dispatch_read_data(
        &mut self,
        val: DbRef,
        db_tp: u16,
        little_endian: bool,
        data: Vec<u8>,
        n: usize,
    ) {
        let actual = data.len();
        let is_text = self.database.is_text_type(db_tp);
        if is_text {
            let s = unsafe { String::from_utf8_unchecked(data) };
            *self
                .database
                .store_mut(&val)
                .addr_mut::<String>(val.rec, val.pos) = s;
        } else if actual == n {
            if let Parts::Vector(_) = &self.database.types[db_tp as usize].parts {
                let vec_ref = *self.database.store(&val).addr::<DbRef>(val.rec, val.pos);
                self.database
                    .write_data(&vec_ref, db_tp, little_endian, &data);
            } else if matches!(
                &self.database.types[db_tp as usize].parts,
                Parts::Byte(_, _) | Parts::Short(_, _) | Parts::ShortRaw(_, _) | Parts::Int(_, _)
            ) {
                // Narrow-integer reads (byte/short/int) target a u8/u16/i32
                // variable whose stack slot is 8 bytes (Phase 2c integer
                // width) and holds a raw i64 — not the +1-encoded form that
                // Parts::Byte/Short use in struct fields, nor the i32::MIN
                // null sentinel Parts::Int uses.  Decode the bytes and store
                // as a sign-extended i64.
                let v: i64 = match &self.database.types[db_tp as usize].parts {
                    Parts::Byte(_, _) => i64::from(data[0]),
                    Parts::Short(_, _) | Parts::ShortRaw(_, _) => {
                        let d: [u8; 2] = data[0..2].try_into().unwrap();
                        if little_endian {
                            i64::from(u16::from_le_bytes(d))
                        } else {
                            i64::from(u16::from_be_bytes(d))
                        }
                    }
                    Parts::Int(_, _) => {
                        let d: [u8; 4] = data[0..4].try_into().unwrap();
                        if little_endian {
                            i64::from(i32::from_le_bytes(d))
                        } else {
                            i64::from(i32::from_be_bytes(d))
                        }
                    }
                    _ => unreachable!(),
                };
                self.database.store_mut(&val).set_int(val.rec, val.pos, v);
            } else {
                self.database.write_data(&val, db_tp, little_endian, &data);
            }
        }
    }

    pub fn read_file(&mut self) {
        let bytes = *self.get_stack::<i64>() as i32;
        let val = *self.get_stack::<DbRef>();
        let file = *self.get_stack::<DbRef>();
        let db_tp = self.code::<u16>();
        if file.rec == 0 {
            return;
        }
        let format = self
            .database
            .store(&file)
            .get_byte(file.rec, file.pos + 32, 0);
        // format: 1=TextFile, 2=LittleEndian, 3=BigEndian, 5=NotExists.
        if format != 1 && format != 5 && format != 2 && format != 3 {
            return;
        }
        let little_endian = format == 2;
        let raw_next = self.database.store(&file).get_long(file.rec, file.pos + 16);
        let next_pos = if raw_next == i64::MIN { 0 } else { raw_next };
        self.database
            .store_mut(&file)
            .set_long(file.rec, file.pos + 8, next_pos);
        let n = bytes as usize;
        #[cfg(feature = "wasm")]
        {
            // FS-D: seek JS cursor to position then read n bytes.
            let file_path = {
                let store = self.database.store(&file);
                store
                    .get_str(store.get_u32_raw(file.rec, file.pos + 24))
                    .to_owned()
            };
            crate::wasm::host_fs_seek(&file_path, next_pos);
            if let Some(data) = crate::wasm::host_fs_read_bytes(&file_path, n) {
                let actual = data.len();
                self.database.store_mut(&file).set_long(
                    file.rec,
                    file.pos + 16,
                    next_pos + actual as i64,
                );
                self.dispatch_read_data(val, db_tp, little_endian, data, n);
            }
        }
        #[cfg(not(feature = "wasm"))]
        {
            let f_nr = self.database.files.len() as i32;
            // #255 / @PLN9: resolve against the program anchor before borrowing
            // the store mutably (resolve_path needs a shared borrow).
            let resolved_name = {
                let s = self.database.store(&file);
                let raw = s.get_str(s.get_u32_raw(file.rec, file.pos + 24)).to_owned();
                self.database.resolve_path(&raw)
            };
            let store = self.database.store_mut(&file);
            let mut file_ref = store.get_i32_raw(file.rec, file.pos + 28);
            if file_ref == i32::MIN {
                match File::open(&resolved_name) {
                    Ok(mut f) => {
                        // apply stored seek position on first open.
                        if next_pos != 0 {
                            let _ = f.seek(SeekFrom::Start(next_pos as u64));
                        }
                        store.set_i32_raw(file.rec, file.pos + 28, f_nr);
                        self.database.files.push(Some(f));
                        file_ref = f_nr;
                    }
                    Err(e) => {
                        // A failed open must NOT leave a dangling ref — indexing
                        // `files[]` with it panicked.  Warn and skip the read (the
                        // recoverable-fault posture), mirroring both the write
                        // path's create fix and the native runtime
                        // (`file_handle_read` → i32::MIN → return).
                        eprintln!("file open error for {resolved_name:?}: {e}");
                        return;
                    }
                }
            } else if let Some(Some(f)) = self.database.files.get_mut(file_ref as usize) {
                // Handle already open: user may have set #next explicitly to seek.
                // Sync the OS file handle with the stored next_pos.
                let _ = f.seek(SeekFrom::Start(next_pos as u64));
            }
            let is_text = self.database.is_text_type(db_tp);
            let mut data = vec![0u8; n];
            let actual = if let Some(Some(f)) = self.database.files.get_mut(file_ref as usize) {
                if is_text {
                    f.read(&mut data).unwrap_or_else(|e| {
                        eprintln!("file read error: {e}");
                        0
                    })
                } else if f.read_exact(&mut data).is_ok() {
                    n
                } else {
                    0
                }
            } else {
                0
            };
            self.database.store_mut(&file).set_long(
                file.rec,
                file.pos + 16,
                next_pos + actual as i64,
            );
            if is_text {
                data.truncate(actual);
            }
            self.dispatch_read_data(val, db_tp, little_endian, data, n);
        }
    }

    pub fn seek_file(&mut self) {
        let pos = *self.get_stack::<i64>();
        let file = *self.get_stack::<DbRef>();
        if file.rec == 0 {
            return;
        }
        #[cfg(feature = "wasm")]
        {
            // FS-D: seek JS-side cursor and update store #next position.
            let file_path = {
                let store = self.database.store(&file);
                store
                    .get_str(store.get_u32_raw(file.rec, file.pos + 24))
                    .to_owned()
            };
            crate::wasm::host_fs_seek(&file_path, pos);
            self.database
                .store_mut(&file)
                .set_long(file.rec, file.pos + 16, pos);
        }
        #[cfg(not(feature = "wasm"))]
        {
            let file_ref = self
                .database
                .store(&file)
                .get_i32_raw(file.rec, file.pos + 28);
            if file_ref == i32::MIN {
                // File not yet open — store the seek position in #next so the first
                // read/write applies it after opening the file.
                self.database
                    .store_mut(&file)
                    .set_long(file.rec, file.pos + 16, pos);
            } else if let Some(f) = &mut self.database.files[file_ref as usize]
                && let Err(e) = f.seek(SeekFrom::Start(pos as u64))
            {
                eprintln!("file seek error: {e}");
            }
        }
    }

    pub fn size_file(&mut self) {
        let file = *self.get_stack::<DbRef>();
        if file.rec == 0 {
            self.put_stack(i64::MIN);
            return;
        }
        #[cfg(feature = "wasm")]
        {
            // FS-D: get file size from JS host bridge.
            let file_path = {
                let store = self.database.store(&file);
                store
                    .get_str(store.get_u32_raw(file.rec, file.pos + 24))
                    .to_owned()
            };
            let size = crate::wasm::host_fs_file_size(&file_path);
            self.put_stack(if size < 0 { i64::MIN } else { size });
        }
        #[cfg(not(feature = "wasm"))]
        {
            let store = self.database.store(&file);
            let file_path = store
                .get_str(store.get_u32_raw(file.rec, file.pos + 24))
                .to_owned();
            // #255 / @PLN9: re-home against the program anchor.
            let file_path = self.database.resolve_path(&file_path);
            let size = if let Ok(meta) = std::fs::metadata(&file_path) {
                meta.len() as i64
            } else {
                i64::MIN
            };
            self.put_stack(size);
        }
    }

    pub fn sync_file(&mut self) {
        let file = *self.get_stack::<DbRef>();
        if file.rec == 0 {
            self.put_stack(false);
            return;
        }
        #[cfg(feature = "wasm")]
        {
            let _ = file;
            self.put_stack(false);
        }
        #[cfg(not(feature = "wasm"))]
        {
            let file_ref = self
                .database
                .store(&file)
                .get_i32_raw(file.rec, file.pos + 28);
            let ok = if file_ref != i32::MIN
                && (file_ref as usize) < self.database.files.len()
                && let Some(f) = &self.database.files[file_ref as usize]
            {
                f.sync_data().is_ok()
            } else {
                true
            };
            self.put_stack(ok);
        }
    }

    pub fn truncate_file(&mut self) {
        let size = *self.get_stack::<i64>();
        let file = *self.get_stack::<DbRef>();
        if file.rec == 0 {
            self.put_stack(false);
            return;
        }
        #[cfg(feature = "wasm")]
        {
            // FS-D: truncate by reading current content, slicing, and rewriting.
            let file_path = {
                let store = self.database.store(&file);
                store
                    .get_str(store.get_u32_raw(file.rec, file.pos + 24))
                    .to_owned()
            };
            let ok = if let Some(mut bytes) = crate::wasm::host_fs_read_binary(&file_path) {
                let new_len = size.max(0) as usize;
                bytes.truncate(new_len);
                bytes.resize(new_len, 0);
                crate::wasm::host_fs_write_binary(&file_path, &bytes) == 0
            } else {
                false
            };
            self.put_stack(ok);
        }
        #[cfg(not(feature = "wasm"))]
        {
            let path = {
                let store = self.database.store(&file);
                store
                    .get_str(store.get_u32_raw(file.rec, file.pos + 24))
                    .to_owned()
            };
            // #255 / @PLN9: re-home against the program anchor.
            let path = self.database.resolve_path(&path);
            // Close any open handle: the handle may be in read or write mode with a stale
            // position, and after resize the position might be beyond the new end of file.
            let file_ref = self
                .database
                .store(&file)
                .get_i32_raw(file.rec, file.pos + 28);
            if file_ref != i32::MIN && (file_ref as usize) < self.database.files.len() {
                self.database.files[file_ref as usize] = None;
                self.database
                    .store_mut(&file)
                    .set_i32_raw(file.rec, file.pos + 28, i32::MIN);
                self.database
                    .store_mut(&file)
                    .set_long(file.rec, file.pos + 8, i64::MIN);
                self.database
                    .store_mut(&file)
                    .set_long(file.rec, file.pos + 16, i64::MIN);
            }
            let ok = OpenOptions::new()
                .write(true)
                .open(&path)
                .and_then(|f| f.set_len(size as u64))
                .is_ok();
            self.put_stack(ok);
        }
    }

    pub fn free_ref(&mut self) {
        let db = *self.get_stack::<DbRef>();
        self.free_ref_db(db);
    }

    /// Plan-57 store-identity gate (`OpFreeRefTag`): pop the DbRef, read the
    /// allocation-site `tag` operand, verify it against the store's recorded tag
    /// (a mismatch means a wrong-store / cross-owner free that `free_named` would
    /// otherwise silently no-op), then free.  Verification build only.
    ///
    /// # Panics
    /// Panics (release included) on a store-tag mismatch — a wrong-store /
    /// cross-owner free.  Only reachable when the `LOFT_STORE_TAG` gate emitted
    /// this op (a deliberate testing build); the op is absent otherwise.
    pub fn free_ref_tag(&mut self) {
        let db = *self.get_stack::<DbRef>();
        let tag = u32::from(self.code::<u16>());
        if db.store_nr != u16::MAX && (db.store_nr as usize) < self.database.allocations.len() {
            let st = &self.database.allocations[db.store_nr as usize];
            // Hard assert (not debug_assert): this op only exists when the
            // LOFT_STORE_TAG gate emitted it — a deliberate testing build — so it
            // must fire in release too. Zero cost when the gate is off (op absent).
            // `tag == 0` covers a slot reused by a NON-`OpDatabase` allocator (e.g.
            // `file()`) after a tagged store freed it — `free_named` resets the tag
            // to 0, so the stale tag never false-positives.  What remains a hard
            // error: a free of a REUSED store re-tagged by a different `OpDatabase`
            // owner (the wrong-store / cross-owner free this gate exists to catch).
            assert!(
                st.free || st.tag == 0 || st.tag == tag,
                "store-tag mismatch on free: store #{} has tag {} but OpFreeRefTag expected {}",
                db.store_nr,
                st.tag,
                tag,
            );
        }
        self.free_ref_db(db);
    }

    /// Plan-57 store-identity gate (`OpStoreTag`): pop the DbRef, read the
    /// allocation-site `tag` operand, stamp it on the store so a later
    /// `OpFreeRefTag` can verify the free corresponds.  Verification build only.
    pub fn store_tag(&mut self) {
        let db = *self.get_stack::<DbRef>();
        let tag = u32::from(self.code::<u16>());
        if db.store_nr != u16::MAX && (db.store_nr as usize) < self.database.allocations.len() {
            self.database.allocations[db.store_nr as usize].tag = tag;
        }
    }

    /// The body of [`free_ref`], factored so [`free_ref_tag`] reuses it after the
    /// DbRef is already popped (the tag operand sits between the popped ref and the
    /// free in the tagged op).
    pub fn free_ref_db(&mut self, db: DbRef) {
        // S37: coroutine DbRefs use store_nr == COROUTINE_STORE (u16::MAX).
        // database.free() is a no-op for this sentinel.  Free the coroutine frame
        // explicitly so that text_owned, stack_bytes, and call_frames are released
        // when a `for` loop exits early (before the generator exhausts).
        if db.store_nr == super::COROUTINE_STORE {
            self.free_coroutine(db.rec as usize);
            return;
        }
        // Plan-57 Phase C: single-ownership (ref-count removed) — close the OS file
        // handle whenever its File store is freed (free_named frees unconditionally).
        #[cfg(not(feature = "wasm"))]
        if db.store_nr != u16::MAX
            && (db.store_nr as usize) < self.database.allocations.len()
            && !self.database.allocations[db.store_nr as usize].free
            && db.rec != 0
            && let Some(&file_type) = self.database.names.get("File")
        {
            let stored_type = self.database.store(&db).get_u32_raw(db.rec, 4) as u16;
            if stored_type == file_type {
                let file_ref = self.database.store(&db).get_i32_raw(db.rec, db.pos + 28);
                if file_ref != i32::MIN && (file_ref as usize) < self.database.files.len() {
                    self.database.files[file_ref as usize] = None;
                }
            }
        }
        self.database.free(&db);
    }

    pub fn format_database(&mut self) {
        let pos = self.code::<u16>();
        let s = self.format_db();
        // @PLAN53 cluster 2 / S4 (2d): format_db pops the composite value's 12-byte
        // DbRef, which advances TOS by stack_step(12) = 16 under LOFT_ALIGN (12 off).
        // The destination-buffer offset must back up by the SAME stepped width, else
        // the String slot is read 4 bytes low and the composite renders empty.  The
        // DbRef (size_ref=12) is the only non-8-multiple here, so it is the lone
        // divergent term.  Identity flag-OFF (stack_step(12) == 12).
        let off = pos - self.stack_step(size_ref()) as u16;
        self.string_mut(off).push_str(&s);
    }

    pub fn format_stack_database(&mut self) {
        let pos = self.code::<u16>();
        let s = self.format_db();
        // @PLAN53 cluster 2 / S4 (2d): see format_database — back up by the stepped
        // DbRef width so the destination String slot is read on its 8-byte boundary.
        let off = pos - self.stack_step(size_ref()) as u16;
        self.string_ref_mut(off).push_str(&s);
    }

    pub fn sizeof_ref(&mut self) {
        let db = *self.get_stack::<DbRef>();
        let new_value: i64 = if db.rec == 0 {
            0
        } else {
            let db_tp = self.database.store(&db).get_int(db.rec, 4) as u16;
            i64::from(self.database.size(db_tp))
        };
        self.put_stack(new_value);
    }

    pub(super) fn format_db(&mut self) -> String {
        let db_tp = self.code::<u16>();
        let format = self.code::<u8>();
        let val = *self.get_stack::<DbRef>();
        // validate DbRef and known_type before raw pointer access
        #[cfg(debug_assertions)]
        {
            assert!(
                val.store_nr == u16::MAX
                    || (val.store_nr as usize) < self.database.allocations.len(),
                "format_db: store_nr={} out of range (allocations.len={}), \
                 rec={}, pos={}, db_tp={}",
                val.store_nr,
                self.database.allocations.len(),
                val.rec,
                val.pos,
                db_tp
            );
            assert!(
                (db_tp as usize) < self.database.types.len(),
                "format_db: db_tp={} out of range (types.len={}), \
                 store_nr={}, rec={}, pos={}",
                db_tp,
                self.database.types.len(),
                val.store_nr,
                val.rec,
                val.pos
            );
        }
        let mut s = String::new();
        ShowDb {
            stores: &self.database,
            store: val.store_nr,
            rec: val.rec,
            pos: val.pos,
            known_type: db_tp,
            pretty: format & 1 > 0,
            json: format & 2 > 0,
            loft: false,
            dump: false,
            compact: false,
            max_depth: u16::MAX,
            max_elements: u16::MAX,
        }
        .write(&mut s, 0);
        s
    }

    pub fn database(&mut self) {
        let var = self.code::<u16>();
        let db_tp = self.code::<u16>();
        let code_pos = self.code_pos;
        // B2-runtime: for EnumValue types, the record must be large enough
        // for the parent enum's largest variant.  The parent's size covers
        // all variants; the variant's own size may be smaller (unit variants).
        let size = self.database.enum_parent_size(db_tp);
        let mut db = *self.get_var::<DbRef>(var);
        if std::env::var("LOFT_TRACE_DB").is_ok() {
            eprintln!(
                "[db] OpDatabase var={var} db_tp={db_tp} db=#{}@{},{} size={size}",
                db.store_nr, db.rec, db.pos,
            );
        }
        // @PLAN51 Cluster II Step 2 — handle u16::MAX sentinel inputs.
        // Mirrors the native runtime (`codegen_runtime.rs::OpDatabase`
        // line 214) so the bytecode VM's `database` opcode accepts the
        // same sentinel a caller-side `OpInitRefSentinel` reset
        // produces.  Without this, `clear()` below would OOB into
        // `allocations[u16::MAX as usize]`.
        if db.store_nr == u16::MAX {
            db = self.database.null();
            *self.mut_var::<DbRef>(var) = db;
        }
        self.database.clear(&db);
        // The record layout is: word 0 = size header (4 B) + type tag (4 B),
        // payload from byte 8.  `Stores::claim` takes WORDS, so the payload's
        // `size` BYTES need `1 + ceil(size / 8)` words — passing `size` raw
        // under-claims for 1-byte payloads (a single-boolean struct/closure
        // record got zero payload bytes and wrote into the next word).
        let r = self.database.claim(&db, 1 + u32::from(size).div_ceil(8));
        self.database.allocations[r.store_nr as usize].created_at = code_pos;
        // P259 commit 3 — record the type allocated into this store so
        // free_named can recognise closure-record stores at free time
        // (cascade-free walks `__closure_*` records' DbRef fields).
        self.database.allocations[r.store_nr as usize].known_type = db_tp;
        self.database
            .store_mut(&r)
            .set_u32_raw(r.rec, 4, u32::from(db_tp));
        self.database.set_default_value(db_tp, &r);
        let db = self.mut_var::<DbRef>(var);
        db.store_nr = r.store_nr;
        db.rec = 1;
        db.pos = 8;
    }

    pub fn new_record(&mut self) {
        let parent_tp = self.code::<u16>();
        let fld = self.code::<u16>();
        let data = *self.get_stack::<DbRef>();
        let new_value = self.database.record_new(&data, parent_tp, fld);
        self.database.set_default_value(
            if fld == u16::MAX {
                self.database.content(parent_tp)
            } else {
                self.database
                    .content(self.database.field_type(parent_tp, fld))
            },
            &new_value,
        );
        self.put_stack(new_value);
    }

    pub fn get_record(&mut self) {
        let (db_tp, key) = self.read_key(false);
        let data = *self.get_stack::<DbRef>();
        let res = if data.rec == 0 {
            DbRef {
                store_nr: data.store_nr,
                rec: 0,
                pos: 0,
            }
        } else {
            self.database.find(&data, db_tp, &key)
        };
        self.put_stack(res);
    }

    /**
    Iterate through a data structure from a given key to a given end-key.
    # Panics
    When called on a not implemented data-structure
    */
    #[allow(clippy::too_many_lines)]
    pub fn iterate(&mut self) {
        let on = self.code::<u8>();
        let arg = self.code::<u16>();
        let keys_size = self.code::<u8>();
        let mut keys = Vec::new();
        for _ in 0..keys_size {
            keys.push(Key {
                type_nr: self.code::<i8>(),
                position: self.code::<u16>(),
            });
        }
        let from_key = self.code::<u8>();
        let till_key = self.code::<u8>();
        let till = self.stack_key(till_key, &keys);
        let from = self.stack_key(from_key, &keys);
        let data = *self.get_stack::<DbRef>();
        // Start the loop at the 'till' key and walk to the 'from' key
        let reverse = on & 64 != 0;
        // The 'till' key is exclusive the found key
        let ex = on & 128 == 0;
        let start;
        let finish;
        let all = &self.database.allocations;
        let trace_iter = std::env::var("LOFT_ITERATE_TRACE").is_ok();
        match on & 63 {
            1 => {
                // index points to the record position inside the store
                if reverse {
                    // for reverse, start must be ONE PAST the last
                    // element to visit (so previous(start) = last element).
                    // finish must be ONE BEFORE the first element to visit
                    // (so when n == finish, iteration is done).
                    // This mirrors the forward case where start is one before
                    // the first and finish is the last to visit.
                    let store = crate::keys::store(&data, all);
                    let till_node = tree::find(&data, true, arg, all, &keys, &till);
                    // till_node = previous(first_node >= till).
                    // For exclusive [lo..hi), till_node IS the last element to visit.
                    // We need start = next(till_node) so previous(start) = till_node.
                    start = if ex {
                        tree::next(store, &new_ref(&data, till_node, arg))
                    } else {
                        // Inclusive: till_node = previous(till_match), need next of till_match
                        let till_match = tree::find(&data, false, arg, all, &keys, &till);
                        tree::next(store, &new_ref(&data, till_match, arg))
                    };
                    // finish = previous(first_from_node) — same as forward start
                    finish = tree::find(&data, true, arg, all, &keys, &from);
                } else {
                    start = tree::find(&data, true, arg, all, &keys, &from);
                    let t = tree::find(&data, ex, arg, all, &keys, &till);
                    finish = if ex {
                        t
                    } else {
                        tree::previous(crate::keys::store(&data, all), &new_ref(&data, t, arg))
                    };
                }
                if trace_iter {
                    eprintln!(
                        "[iterate] on=index reverse={reverse} ex={ex} start={start} finish={finish} from_keys={} till_keys={}",
                        from.len(),
                        till.len()
                    );
                }
            }
            2 => {
                // sorted points to the position of the record inside the vector
                // empty from/till arrays signal "no constraint on this side".
                // Phase 2c: sorted collection pointer is a 4-byte u32 — using
                // `get_int` (8 bytes post-2c) overflows into the next field.
                // get_i32_raw returns i32::MIN for unresolved-type fields;
                // guard against negative values (0 = empty, i32::MIN = unresolved).
                let sorted_rec_raw = all[data.store_nr as usize].get_i32_raw(data.rec, data.pos);
                let sorted_rec = if sorted_rec_raw <= 0 {
                    0
                } else {
                    sorted_rec_raw as u32
                };
                let vec_len = if sorted_rec == 0 {
                    0
                } else {
                    all[data.store_nr as usize].get_u32_raw(sorted_rec, 4)
                };
                if reverse {
                    start = if till.is_empty() {
                        // no upper bound → start past the last element
                        vec_len
                    } else {
                        vector::sorted_find(&data, ex, arg, all, &keys, &till).0 + u32::from(!ex)
                    };
                    finish = if from.is_empty() {
                        // no lower bound → finish at 0 (visit all elements down to first)
                        0
                    } else {
                        vector::sorted_find(&data, ex, arg, all, &keys, &from).0 + 1
                    };
                } else {
                    let s = if from.is_empty() {
                        0
                    } else {
                        vector::sorted_find(&data, true, arg, all, &keys, &from).0
                    };
                    start = if s == 0 { u32::MAX } else { s - 1 };
                    finish = if till.is_empty() {
                        vec_len
                    } else {
                        let (t, cmp) = vector::sorted_find(&data, ex, arg, all, &keys, &till);
                        if ex || cmp { t } else { t + 1 }
                    };
                }
                if trace_iter {
                    eprintln!(
                        "[iterate] on=sorted reverse={reverse} ex={ex} sorted_rec={sorted_rec} \
                         vec_len={vec_len} start={start} finish={finish}"
                    );
                }
            }
            3 => {
                // ordered points to the position inside the vector of references
                if from.is_empty() && till.is_empty() {
                    // C60 piece 3 edit E: unbounded iteration (`for e
                    // in h { … }` with no range).  ordered_find with an
                    // empty key returns (0, true) which collapses
                    // start=0/finish=0, so the step protocol never
                    // fires even once.  Set start to the "not started"
                    // sentinel that `vector_next` recognises at
                    // src/vector.rs:455 — it checks `*pos == i32::MAX`
                    // (the i32 positive max, 0x7FFF_FFFF), *not*
                    // u32::MAX.  Passing u32::MAX here casts to i32 as
                    // -1 and falls into the "advance" branch, reading
                    // garbage at pos=-1+size.
                    start = i32::MAX as u32;
                    finish = 0;
                } else if reverse {
                    start = vector::ordered_find(&data, true, all, &keys, &from).0 + u32::from(!ex);
                    finish = vector::ordered_find(&data, ex, all, &keys, &till).0 + 1;
                } else {
                    let (s, cmp) = vector::ordered_find(&data, ex, all, &keys, &till);
                    start = if cmp || s == 0 { s } else { s - 1 };
                    finish = vector::ordered_find(&data, ex, all, &keys, &from).0 - u32::from(!ex);
                }
            }
            _ => panic!("Not implemented on {on}"),
        }
        // @PLAN53 cluster 2 / S4 (2b+2c): push the iterator state as ONE
        // contiguous 8-byte value, not two separate u32s.  The consumer stores
        // it into a single I64 var (`{id}#iter_state`, "cur<<32|finish") via an
        // i64 read.  Under LOFT_ALIGN each `put_stack::<u32>` advances a stepped
        // (8-byte) slot, so two pushes leave start@P and finish@P+8 with a 4-byte
        // gap — the i64 read-back then takes [finish | padding] and drops `start`,
        // killing the sorted cursor (2b) and shifting the hash cursor (2c).  A
        // single u64 push writes the 8 bytes contiguously in BOTH modes; the
        // little-endian layout (low=start, high=finish) is byte-identical to the
        // old two-u32 push when the step is identity (flag-OFF).
        self.put_stack((u64::from(finish) << 32) | u64::from(start));
    }

    pub(super) fn stack_key(&mut self, size: u8, keys: &[Key]) -> Vec<Content> {
        let mut key = Vec::new();
        for (k_nr, k) in keys.iter().enumerate() {
            if k_nr >= size as usize {
                break;
            }
            match k.type_nr.abs() {
                1 => key.push(Content::Long(*self.get_stack::<i64>())),
                2 => key.push(Content::Long(*self.get_stack::<i64>())),
                3 => key.push(Content::Single(*self.get_stack::<f32>())),
                4 => key.push(Content::Float(*self.get_stack::<f64>())),
                5 => key.push(Content::Long(i64::from(*self.get_stack::<bool>()))),
                6 => key.push(Content::Str(self.string())),
                7 => key.push(Content::Long(i64::from(*self.get_stack::<u8>()))),
                _ => panic!("Unknown key type"),
            }
        }
        key
    }

    /**
    Step to the next value for the iterator.
    # Panics
    When requested on a not-implemented iterator.
    */
    pub fn step(&mut self) {
        let state_var = self.code::<u16>();
        let on = self.code::<u8>();
        let arg = self.code::<u16>();
        let cur = *self.get_var::<u32>(state_var);
        let finish = *self.get_var::<u32>(state_var - 4);
        let reverse = on & 64 != 0;
        let data = *self.get_stack::<DbRef>();
        let store = crate::keys::store(&data, &self.database.allocations);
        let cur = if data.rec == 0 || finish == u32::MAX {
            new_ref(&data, 0, 0)
        } else {
            match on & 63 {
                1 => {
                    let rec = new_ref(&data, cur, arg);
                    let n = if cur == 0 {
                        if reverse {
                            tree::last(&data, arg, &self.database.allocations).rec
                        } else {
                            tree::first(&data, arg, &self.database.allocations).rec
                        }
                    } else if reverse {
                        tree::previous(store, &rec)
                    } else {
                        tree::next(store, &rec)
                    };
                    self.put_var(state_var - 8, n);
                    if std::env::var("LOFT_ITERATE_TRACE").is_ok() {
                        eprintln!(
                            "[step] on=index reverse={reverse} cur={cur} -> n={n} finish={finish} done={}",
                            n == finish
                        );
                    }
                    if n == finish {
                        self.put_var(state_var - 12, u32::MAX);
                    }
                    new_ref(&data, n, 8)
                }
                2 => {
                    let mut pos = if cur == u32::MAX {
                        i32::MAX
                    } else {
                        cur as i32
                    };
                    if reverse {
                        // `iterate()` sets start > length for the "not started" sentinel
                        // (pos >= length is treated as past-the-end in vector_step_rev).
                        vector::vector_step_rev(&data, &mut pos, &self.database.allocations);
                        self.put_var(state_var - 8, pos as u32);
                        if pos == i32::MAX {
                            self.put_var(state_var - 12, u32::MAX);
                        }
                    } else {
                        vector::vector_step(&data, &mut pos, &self.database.allocations);
                        self.put_var(state_var - 8, pos as u32);
                        if pos as u32 >= finish {
                            pos = i32::MAX;
                            self.put_var(state_var - 12, u32::MAX);
                        }
                    }
                    self.database.element_reference(
                        &data,
                        if pos == i32::MAX {
                            i32::MAX
                        } else {
                            8 + pos * i32::from(arg)
                        },
                    )
                }
                3 => {
                    let mut pos = cur as i32;
                    vector::vector_next(&data, &mut pos, 4, &self.database.allocations);
                    let vector = store.get_u32_raw(data.rec, data.pos);
                    let rec = if pos == i32::MAX {
                        0
                    } else {
                        store.get_u32_raw(vector, pos as u32)
                    };
                    self.put_var(state_var - 8, pos as u32);
                    DbRef {
                        store_nr: data.store_nr,
                        rec,
                        pos: 8,
                    }
                }
                _ => panic!("Not implemented"),
            }
        };
        self.put_stack(cur);
    }

    /**
    Remove the current value from the iterator. Move the iterator to the previous value.
    # Panics
    When requested on a not-implemented iterator.
    */
    pub fn remove(&mut self) {
        let state_var = self.code::<u16>();
        let on = self.code::<u8>();
        let tp = self.code::<u16>();
        let reverse = on & 64 != 0;
        let cur = *self.get_var::<i32>(state_var);
        let data = *self.get_stack::<DbRef>();
        // Defense-in-depth: coroutine DbRefs (store_nr == u16::MAX) must not reach remove().
        // The compiler already rejects e#remove on generator iterators (CO1.5c / S24), so this
        // guard only fires if that check is somehow bypassed — preventing release-build corruption.
        assert!(
            data.store_nr != u16::MAX,
            "e#remove on coroutine DbRef — compiler check should have rejected this"
        );
        if data.store_nr == u16::MAX {
            return;
        }
        match on & 63 {
            0 => {
                // vector
                let n = if reverse { cur + 1 } else { cur - 1 };
                vector::remove_vector(
                    &data,
                    u32::from(self.database.size(tp)),
                    i64::from(cur),
                    &mut self.database.allocations,
                );
                // iter_var (#index) is i64 on stack post-2c.  Sign-extend n
                // (may be −1 after remove-at-index-0) and write all 8 bytes
                // so the high word doesn't leak a stale value into the next
                // VarInt — otherwise −1 comes back as 0xFFFFFFFF = 2^32−1.
                // put_var adds size_of::<T>(); switching i32→i64 shifts the
                // target address up by 4 bytes, so pos moves from −8 to −4.
                // @PLAN53 cluster 2 / 2f: this is the lone non-self-compensating
                // delta in remove() — an i64 put (step(8)=8 both modes) after the
                // DbRef pop (step(12): 12 V1, 16 V2).  The post-pop 4-byte puts
                // self-compensate (step(12)−step(4) cancels); this i64 one does not,
                // so the raw −4 lands 4 bytes low under V2 and stomps the loop's
                // vector-variable slot → next iteration reads a garbage DbRef
                // (store_nr=65535) → panic in keys.rs/get_vector.  Thread the
                // step(12) term: −(step(12)−8) = −4 off (identity), −8 aligned.
                self.put_var(state_var - (self.stack_step(12) - 8) as u16, i64::from(n));
            }
            1 => {
                // Use the outer `cur` (read as i32 before the data DbRef was popped).
                // Re-reading `state_var` here would give the wrong slot because `get_stack`
                // already consumed the 12-byte DbRef, shifting the effective slot offset.
                let cur = cur as u32;
                if cur == u32::MAX {
                    return;
                }
                // tp is now the Index type index (not fields_offset); get the actual
                // fields byte offset for tree navigation.
                let fields = self.database.fields(tp);
                let cur_ref = new_ref(&data, cur, fields);
                // Compute n_after = in-order successor (or predecessor for reverse) of cur
                // BEFORE removing cur, so the tree pointers are still intact.
                let n_after = {
                    let store = crate::keys::store(&data, &self.database.allocations);
                    if reverse {
                        tree::previous(store, &cur_ref)
                    } else {
                        tree::next(store, &cur_ref)
                    }
                };
                self.database.remove(&data, &cur_ref, tp);
                if n_after == 0 {
                    // Removed the last element in iteration order; signal end-of-iteration
                    // by overwriting the finish slot (same as step's put_var(state_var-12)).
                    self.put_var(state_var - 12, u32::MAX);
                } else {
                    // Set slot = predecessor of n_after in the modified tree so the next
                    // step() call computes next(pred) = n_after and visits n_after.
                    let pred = {
                        let store = crate::keys::store(&data, &self.database.allocations);
                        let n_ref = new_ref(&data, n_after, fields);
                        if reverse {
                            tree::next(store, &n_ref)
                        } else {
                            tree::previous(store, &n_ref)
                        }
                    };
                    self.put_var(state_var - 8, pred);
                    // If n_after is the finish boundary, also signal end-of-iteration.
                    // @PLAN53 cluster 2 / 2f: get_var adds NO step, so this finish-read
                    // delta must thread step(12) the other way: −(step(12)+4) = −16 off
                    // (identity), −20 aligned.  Latent (no test exercises case-1 index
                    // #remove under V2 yet) but the same disease as the case-0 fix above.
                    let finish = *self.get_var::<u32>(state_var - (self.stack_step(12) + 4) as u16);
                    if n_after == finish {
                        self.put_var(state_var - 12, u32::MAX);
                    }
                }
            }
            2 => {
                // sorted: tp is the element size in bytes (from loop_db_tp)
                if cur < 0 {
                    return;
                }
                let n = if reverse { cur + 1 } else { cur - 1 };
                vector::remove_vector(
                    &data,
                    u32::from(tp),
                    i64::from(cur),
                    &mut self.database.allocations,
                );
                self.put_var(state_var - 8, n);
            }
            3 => {
                // ordered: tp is element size (4 bytes); cur is byte offset (8, 12, ...)
                if cur < 0 {
                    return;
                }
                let size = u32::from(tp);
                let n = if reverse {
                    cur + i32::from(tp)
                } else {
                    cur - i32::from(tp)
                };
                vector::remove_vector(
                    &data,
                    size,
                    i64::from((cur - 8) / i32::from(tp)),
                    &mut self.database.allocations,
                );
                self.put_var(state_var - 8, n);
            }
            _ => panic!("Not implemented on {on}"),
        }
    }

    /**
    Clear the given structure on the field
    */
    pub fn clear(&mut self) {
        let tp = self.code::<u16>();
        let data = *self.get_stack::<DbRef>();
        self.database.remove_claims(&data, tp);
    }

    pub fn append_copy(&mut self) {
        let tp = self.code::<u16>();
        let multiply = *self.get_stack::<i64>() as u32;
        let data = *self.get_stack::<DbRef>();
        let ctp = self.database.content(tp);
        let size = u32::from(self.database.size(ctp));
        let length = vector::length_vector(&data, &self.database.allocations);
        let v_rec =
            crate::keys::store(&data, &self.database.allocations).get_u32_raw(data.rec, data.pos);
        let from = DbRef {
            store_nr: data.store_nr,
            rec: v_rec,
            pos: 8 + (length * size - size),
        };
        vector::vector_append(&data, size, &mut self.database.allocations);
        self.database.vector_set_size(&data, multiply, size);
        for i in 0..(multiply - 1) {
            let to = DbRef {
                store_nr: data.store_nr,
                rec: v_rec,
                pos: 8 + (length + i) * size,
            };
            self.database.copy_block(&from, &to, size);
            self.database.copy_claims(&data, &to, ctp);
            self.database
                .watch_oob_text(&to, ctp, Some(&data), "append_copy");
        }
    }

    pub fn copy_record(&mut self) {
        let raw_tp = self.code::<u16>();
        let to = *self.get_stack::<DbRef>();
        let data = *self.get_stack::<DbRef>();
        self.do_copy_record(data, to, raw_tp);
    }

    /// Core of `OpCopyRecord` / `OpCopyRefOrNull`: deep-copy record `data` into
    /// the already-allocated destination `to`.  `raw_tp`'s high bit (`0x8000`)
    /// frees the source store after the copy (#120).  Factored so the nullable
    /// struct-return ABI-B path (`copy_ref_or_null`) reuses the non-null branch.
    fn do_copy_record(&mut self, data: DbRef, to: DbRef, raw_tp: u16) {
        // Issue #120: high bit of tp signals "free source store after copy".
        let free_source = raw_tp & 0x8000 != 0;
        let tp = raw_tp & 0x7FFF;
        // @PLAN51 Cluster II — true alias copy is a no-op.  When data
        // and to refer to the SAME slot (full DbRef equality), the
        // remove_claims + copy_block + copy_claims sequence would
        // destroy and rebuild the same nested vectors / texts (because
        // remove_claims's prelude frees nested heap records BEFORE
        // copy_block can read them — corrupting same-store callers
        // like extended-S1 inner Sets).  Short-circuit before any
        // destructive op so callers that pass aliased src/dst (e.g.
        // ref_return-promoted callees whose return aliases their
        // hidden buffer arg) get safe semantics.  Mirrors the safety
        // gate the native runtime needs at codegen_runtime.rs:OpCopyRecord.
        if data == to {
            // free_source is suppressed (data.store_nr == to.store_nr
            // is already the existing skip-free condition at line 1225);
            // nothing more to do.
            let _ = free_source;
            return;
        }
        // @PLN25 — a NULL source (the `store_nr == u16::MAX` sentinel) has no
        // store to read; copying from it is meaningless.  `store(&data)` below
        // would index `allocations[65535]` and panic (`allocation.rs:560` OOB).
        // This guards the pre-existing `main` crash on `vec[i] = <runtime-null>`
        // into a non-nullable inline element (where there is no null encoding) —
        // leave the destination unchanged rather than crash.  (For a NULLABLE
        // `__nullable<S>` element the parser emits the discriminant-0 store, not
        // OpCopyRecord, so this path is the can't-represent-null fallback.)
        if data.store_nr == u16::MAX {
            return;
        }
        // cluster-462 tool-gap #1 — name the premature-free op: when the copy
        // SOURCE is read while its store is still freed (the operand-stack stale
        // ref the LOFT_UAF frame scan misses), report the source, the line that
        // freed it, and the copy site.  Gated on LOFT_UAF; capped to avoid flood.
        if crate::keys::uaf_any_enabled()
            && (data.store_nr as usize) < self.database.allocations.len()
            && self.database.allocations[data.store_nr as usize].free
        {
            thread_local! {
                static REPORTED: std::cell::RefCell<std::collections::HashSet<u16>> =
                    std::cell::RefCell::new(std::collections::HashSet::new());
            }
            let first = REPORTED.with(|s| s.borrow_mut().insert(data.store_nr));
            if first {
                let line_of = |ln: &std::collections::BTreeMap<u32, u32>, pc: u32| -> u32 {
                    ln.range(..=pc).next_back().map_or(0, |(_, &v)| v)
                };
                let copy_line = line_of(&self.line_numbers, self.code_pos);
                let copy_d_nr = self.call_stack.last().map_or(u32::MAX, |f| f.d_nr);
                let (free_pc, free_line, free_d_nr, free_op) =
                    crate::keys::uaf_freed_pc(data.store_nr)
                        .map_or((0, 0, u32::MAX, u16::MAX), |(pc, d, op)| {
                            (pc, line_of(&self.line_numbers, pc), d, op)
                        });
                eprintln!(
                    "[uaf-src] OpCopyRecord reads FREED source store #{} (rec={},pos={},tp={tp}) \
                     at copy-site pc={} (line {copy_line}, fn d_nr={copy_d_nr}); that store was \
                     last freed at pc={free_pc} (line {free_line}, fn d_nr={free_d_nr}, op={free_op})",
                    data.store_nr, data.rec, data.pos, self.code_pos,
                );
            }
        }
        // (b) cluster-462 LOFT_UAF_REUSE — the REUSED case the free-check above misses:
        // the source slot is LIVE (free=false) but `validate_claims` finds it structurally
        // invalid for `tp` (out-of-range rec/pos, broken interior edges), i.e. it was
        // freed-then-reused as a DIFFERENT record since the source ref was minted. That is
        // the stale-reused read behind the post-reuse SIGSEGV (#462 @ sim.loft:3546) —
        // named here, before the faulting copy. `validate_claims` is bounds-checked (it
        // reports, never faults). Deduped by store slot.
        if crate::keys::uaf_reuse_enabled()
            && data.rec != 0
            && (data.store_nr as usize) < self.database.allocations.len()
            && !self.database.allocations[data.store_nr as usize].free
        {
            let mut problems = 0u32;
            self.database
                .validate_claims(&data, tp, "uaf-reuse-src", &mut problems);
            if problems > 0 {
                thread_local! {
                    static REPORTED_REUSE: std::cell::RefCell<std::collections::HashSet<u16>> =
                        std::cell::RefCell::new(std::collections::HashSet::new());
                }
                let first = REPORTED_REUSE.with(|s| s.borrow_mut().insert(data.store_nr));
                if first {
                    let line = self
                        .line_numbers
                        .range(..=self.code_pos)
                        .next_back()
                        .map_or(0, |(_, &v)| v);
                    let cur_kt = self.database.allocations[data.store_nr as usize].known_type;
                    eprintln!(
                        "[uaf-reuse] OpCopyRecord source store #{} (rec={},pos={}) is LIVE but \
                         STRUCTURALLY INVALID for tp={tp} ({problems} broken edge(s); slot now \
                         holds known_type={cur_kt}) — freed-then-reused-as-different, a stale read \
                         at copy-site pc={} (line {line})",
                        data.store_nr, data.rec, data.pos, self.code_pos,
                    );
                }
            }
        }
        if std::env::var("LOFT_TRACE_CR").is_ok() {
            let src_w = if data.rec != 0 {
                self.database.store(&data).get_int(data.rec, data.pos)
            } else {
                -1
            };
            let src_data_ptr = if data.rec != 0 {
                self.database
                    .store(&data)
                    .get_u32_raw(data.rec, data.pos + 8)
            } else {
                0
            };
            let src_vec0 = if src_data_ptr != 0 {
                let len = self.database.store(&data).get_u32_raw(src_data_ptr, 4);
                if len > 0 {
                    self.database.store(&data).get_int(src_data_ptr, 8)
                } else {
                    -3
                }
            } else {
                -4
            };
            let dst_w_before = if to.rec != 0 {
                self.database.store(&to).get_int(to.rec, to.pos)
            } else {
                -1
            };
            eprintln!(
                "[cr] OpCopyRecord src=#{}@{},{} (w={src_w} data_ptr={src_data_ptr} vec0={src_vec0}) dst=#{}@{},{} (w_before={dst_w_before}) tp={tp} free_src={free_source}",
                data.store_nr, data.rec, data.pos, to.store_nr, to.rec, to.pos,
            );
        }
        if std::env::var("LOFT_TRACE_CR").is_ok() {
            // #306 — bounds-checked pre-walk of both records' claim graphs;
            // names the first broken interior edge instead of faulting on it.
            let mut problems = 0u32;
            self.database
                .validate_claims(&data, tp, "src", &mut problems);
            self.database.validate_claims(&to, tp, "dst", &mut problems);
            if problems > 0 {
                eprintln!(
                    "[cr-check] {problems} broken edge(s) found before copy \
                     src=#{}.{},{} dst=#{}.{},{} tp={tp}",
                    data.store_nr, data.rec, data.pos, to.store_nr, to.rec, to.pos,
                );
            }
        }
        let code_pos = self.code_pos;
        let size = u32::from(self.database.size(tp));
        // free any nested vectors/strings already owned by the destination
        // before overwriting it, to prevent double-free and leaks when a struct
        // field containing a nested vector is reassigned.
        self.database.remove_claims(&to, tp);
        self.database.copy_block(&data, &to, size);
        self.database.copy_claims(&data, &to, tp);
        // LOFT_WATCH_STORE (cluster-462 write-watch) — name the copy that leaves a garbage
        // text-ptr in the watched store; src_oob says propagated-vs-introduced-here.
        self.database
            .watch_oob_text(&to, tp, Some(&data), "copy_record");
        if std::env::var("LOFT_TRACE_CR").is_ok() {
            let dst_w = self.database.store(&to).get_int(to.rec, to.pos);
            let dst_data_ptr = self.database.store(&to).get_u32_raw(to.rec, to.pos + 8);
            eprintln!(
                "[cr]   AFTER copy: dst=#{}@{},{} w={dst_w} data_ptr={dst_data_ptr}",
                to.store_nr, to.rec, to.pos,
            );
            if dst_data_ptr != 0 {
                let len = self.database.store(&to).get_u32_raw(dst_data_ptr, 4);
                let first = if len > 0 {
                    self.database.store(&to).get_int(dst_data_ptr, 8)
                } else {
                    -2
                };
                eprintln!("[cr]   AFTER copy: dst.vec_rec={dst_data_ptr} len={len} [0]={first}");
            }
        }
        // @P317 — LOFT_LOG=copy_check: warn if the deep copy changed any nested
        // collection length (before the source-free below, so src is intact).
        // Call-site-gated so production pays only a branch-predicted cached
        // read, not a function call, when the mode is off.
        if self.database.copy_check_enabled() {
            self.database
                .report_copy_mismatches(&data, &to, tp, "copy_record");
        }
        // Record which bytecode position performed this deep copy.
        self.database.allocations[to.store_nr as usize].last_op_at = code_pos;
        // Issue #120: free the source store after deep copy when the caller
        // knows the source is a temporary (callee's return store) that would
        // otherwise leak because is_ret_work_ref suppresses its OpFreeRef.
        if free_source
            && data.store_nr != to.store_nr
            && data.store_nr != 0
            && !self.database.allocations[data.store_nr as usize].free
            && !self.database.allocations[data.store_nr as usize].read_only
            && !self.database.allocations[data.store_nr as usize].free_protected
        {
            self.database.free(&data);
        }
    }

    /// `OpCopyRefOrNull` — the ABI-B caller path for `v = call()` where the
    /// callee has a heap parameter and returns a struct-OR-null.  Pops the call
    /// result `src`; the destination variable `pos` already holds a freshly
    /// allocated record (emitted `OpDatabase` just before).  When `src` is the
    /// null sentinel (`store_nr == u16::MAX`), a plain `OpCopyRecord` would index
    /// `allocations[u16::MAX]` and panic — and even guarded it would leave the
    /// pre-allocated record bound, so `v == null` (which tests `rec == 0`) would
    /// read false.  So: free the pre-allocated record and bind the sentinel into
    /// the slot, making `v` genuinely null.  Otherwise deep-copy as usual.
    pub fn copy_ref_or_null(&mut self) {
        let pos = self.code::<u16>();
        let raw_tp = self.code::<u16>();
        let src = *self.get_stack::<DbRef>();
        if src.store_nr == u16::MAX {
            // Null return: reclaim the pre-allocated destination record and bind
            // the null sentinel so `v == null` holds.
            let dst = *self.get_var::<DbRef>(pos);
            if dst.store_nr != u16::MAX {
                self.database.free(&dst);
            }
            *self.mut_var::<DbRef>(pos) = self.database.null();
            return;
        }
        let dst = *self.get_var::<DbRef>(pos);
        self.do_copy_record(src, dst, raw_tp);
    }

    /// @P295 — deep-copy a keyed collection (`sorted`/`hash`/`index`) into a
    /// keyed-collection LOCAL `dest`, replacing its prior contents.  Unlike
    /// `copy_record` there is NO `copy_block`: a keyed local's slot is a
    /// `DbRef` to a dedicated store whose collection header lives at
    /// `(store, 1, 8)`, so `remove_claims(dest)` (free + reset) followed by
    /// `copy_claims(src, dest, tp)` (per-kind rebuild of the bucket/tree
    /// index) is the complete, correct copy.  `tp` is the SPECIFIC keyed
    /// type id; its `0x8000` high bit frees the source store after the copy
    /// (set by the parser for fresh-storage RHS like `s = build()`).
    pub fn replace_keyed(&mut self) {
        let raw_tp = self.code::<u16>();
        let free_source = raw_tp & 0x8000 != 0;
        let tp = raw_tp & 0x7FFF;
        let dest = *self.get_stack::<DbRef>();
        let src = *self.get_stack::<DbRef>();
        self.database.remove_claims(&dest, tp);
        self.database.copy_claims(&src, &dest, tp);
        if self.database.copy_check_enabled() {
            self.database
                .report_copy_mismatches(&src, &dest, tp, "replace_keyed");
        }
        if free_source
            && src.store_nr != dest.store_nr
            && src.store_nr != 0
            && !self.database.allocations[src.store_nr as usize].free
            && !self.database.allocations[src.store_nr as usize].read_only
            && !self.database.allocations[src.store_nr as usize].free_protected
        {
            self.database.free(&src);
        }
    }

    /// @P305 — `coll[key] = value` insert-or-replace into a keyed
    /// collection.  `OpSetKeyed(coll, value, tp)`: `tp`'s `0x8000` bit frees
    /// `value`'s store after the deep copy (caller temp).
    pub fn set_keyed(&mut self) {
        let raw_tp = self.code::<u16>();
        let free_source = raw_tp & 0x8000 != 0;
        let db_tp = raw_tp & 0x7FFF;
        let value = *self.get_stack::<DbRef>();
        let coll = *self.get_stack::<DbRef>();
        self.database.set_keyed(&coll, &value, db_tp, free_source);
    }

    pub fn hash_add(&mut self) {
        let tp = self.code::<u16>();
        let rec = *self.get_stack::<DbRef>();
        let data = *self.get_stack::<DbRef>();
        hash::add(
            &data,
            &rec,
            &mut self.database.allocations,
            &self.database.types[tp as usize].keys,
        );
    }

    pub fn validate(&mut self) {
        let tp = self.code::<u16>();
        let data = *self.get_stack::<DbRef>();
        self.database.validate(&data, tp);
    }

    pub fn hash_find(&mut self) {
        let data = *self.get_stack::<DbRef>();
        let (db_tp, key) = self.read_key(true);
        let res = hash::find(
            &data,
            &self.database.allocations,
            &self.database.types[db_tp as usize].keys,
            &key,
        );
        self.put_stack(res);
    }

    pub fn hash_remove(&mut self) {
        let tp = self.code::<u16>();
        let rec = *self.get_stack::<DbRef>();
        let data = *self.get_stack::<DbRef>();
        if rec.rec != 0 {
            self.database.remove(&data, &rec, tp);
        }
    }

    pub(super) fn read_key(&mut self, full: bool) -> (u16, Vec<Content>) {
        let db_tp = self.code::<u16>();
        let keys = self.database.get_keys(db_tp);
        let no_keys = if full {
            keys.len() as u8
        } else {
            self.code::<u8>()
        };
        let mut key = Vec::new();
        for (k_nr, k) in keys.iter().enumerate() {
            if k_nr >= no_keys as usize {
                break;
            }
            match k {
                0 | 1 => key.push(Content::Long(*self.get_stack::<i64>())),
                2 => key.push(Content::Single(*self.get_stack::<f32>())),
                3 => key.push(Content::Float(*self.get_stack::<f64>())),
                4 => key.push(Content::Long(i64::from(*self.get_stack::<bool>()))),
                5 => key.push(Content::Str(self.string())),
                6 => key.push(Content::Long(i64::from(*self.get_stack::<u32>()))),
                _ => {
                    // Narrow integer storage (Parts::Int / Short / ShortRaw /
                    // Byte) — the lookup value is still an i64 on the stack
                    // even when the field's storage is narrower.  Pop 8
                    // bytes so subsequent stack reads stay aligned; the
                    // catch-all 1-byte pop here was broken silently for
                    // every narrow-key hash since Parts::Int was introduced.
                    match &self.database.types[*k as usize].parts {
                        crate::database::Parts::Int(_, _)
                        | crate::database::Parts::Short(_, _)
                        | crate::database::Parts::ShortRaw(_, _)
                        | crate::database::Parts::Byte(_, _) => {
                            key.push(Content::Long(*self.get_stack::<i64>()));
                        }
                        _ => key.push(Content::Long(i64::from(*self.get_stack::<u8>()))),
                    }
                }
            }
            // We assume that all none-base types are enumerate types.
        }
        (db_tp, key)
    }

    pub fn finish_record(&mut self) {
        let parent_tp = self.code::<u16>();
        let fld = self.code::<u16>();
        let record = *self.get_stack::<DbRef>();
        let data = *self.get_stack::<DbRef>();
        self.database.record_finish(&data, &record, parent_tp, fld);
    }

    pub fn db_from_text(&mut self, val: &str, db_tp: u16) -> DbRef {
        // @P372: size the target record to the struct, not a fixed 8 words.
        // The struct is written at `pos: 8` (after the 8-byte record header),
        // so it needs `8 + size` bytes = `1 + size.div_ceil(8)` words.  The old
        // fixed `database(8)` (64 bytes) left only 56 bytes for the struct, so
        // any struct larger than 56 bytes (e.g. 8 `integer` fields = 64 bytes)
        // overflowed: the tail field's write — notably a vector field's handle
        // at byte 64 — landed on the ADJACENT block's header, zeroing it.  A
        // later `claim` then walked the free list into that zero-sized block and
        // `claim_scan` spun forever (`pos += abs(claim)` never advances; the
        // `debug_assert_ne!` guard is compiled out in release).  A vector field
        // made it deterministic because appending its elements triggers exactly
        // such a `claim`.  Keep a floor of 8 words to preserve the prior slack
        // for small structs.
        let words = std::cmp::max(8, 1 + u32::from(self.database.size(db_tp)).div_ceil(8));
        let db = self.database.database(words);
        let into = DbRef {
            store_nr: db.store_nr,
            rec: db.rec,
            pos: 8,
        };
        self.database.set_default_value(db_tp, &into);
        self.database.last_parse_errors.clear();
        if let Some(err) = self.database.parse(val, db_tp, &into) {
            self.database.last_parse_errors.push(err);
        }
        into
    }

    pub fn insert_vector(&mut self) {
        let size = self.code::<u16>();
        let db_tp = self.code::<u16>();
        let index = *self.get_stack::<i64>();
        let r = *self.get_stack::<DbRef>();
        let new_value =
            vector::insert_vector(&r, u32::from(size), index, &mut self.database.allocations);
        self.database.set_default_value(db_tp, &new_value);
        self.put_stack(new_value);
    }
}
