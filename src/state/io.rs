// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use super::{State, new_ref, size_ref};
use crate::database::{Parts, ShowDb};
use crate::keys::{Content, DbRef, Key};
use crate::{hash, tree, vector};
use std::fs::{File, OpenOptions};
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
        let store = self.database.store(&file);
        let file_path = store.get_str(store.get_int(file.rec, file.pos + 24) as u32);
        let buf = self.database.store_mut(&r).addr_mut::<String>(r.rec, r.pos);
        if let Ok(mut f) = File::open(file_path)
            && f.read_to_string(buf).is_err()
        {
            buf.clear();
        }
    }

    pub fn write_file(&mut self) {
        let val = *self.get_stack::<DbRef>();
        let file = *self.get_stack::<DbRef>();
        let db_tp = *self.code::<u16>();
        if file.rec == 0 {
            return;
        }
        let f_nr = self.database.files.len() as i32;
        let format = self
            .database
            .store(&file)
            .get_byte(file.rec, file.pos + 32, 0);
        // format: 1=TextFile, 2=LittleEndian, 3=BigEndian, 5=NotExists (default to TextFile).
        if format != 1 && format != 5 && format != 2 && format != 3 {
            return;
        }
        let little_endian = format == 2;
        let file_ref = self.database.store(&file).get_int(file.rec, file.pos + 28);
        let file_ref = if file_ref == i32::MIN {
            // Open the file for writing (creates or truncates), same for text and binary.
            let file_name = {
                let store = self.database.store(&file);
                store
                    .get_str(store.get_int(file.rec, file.pos + 24) as u32)
                    .to_owned()
            };
            match File::create(&file_name) {
                Ok(f) => {
                    self.database
                        .store_mut(&file)
                        .set_int(file.rec, file.pos + 28, f_nr);
                    // If the file was marked NotExists, update format to TextFile now that it exists.
                    if format == 5 {
                        self.database
                            .store_mut(&file)
                            .set_byte(file.rec, file.pos + 32, 0, 1);
                    }
                    self.database.files.push(Some(f));
                    f_nr
                }
                Err(e) => {
                    eprintln!("file create error for {file_name:?}: {e}");
                    return;
                }
            }
        } else {
            file_ref
        };
        // Track position: set #index = where this write starts, then advance #next after.
        let raw_next = self.database.store(&file).get_long(file.rec, file.pos + 16);
        let next_pos = if raw_next == i64::MIN { 0 } else { raw_next };
        self.database
            .store_mut(&file)
            .set_long(file.rec, file.pos + 8, next_pos);
        // Assemble the bytes to write.
        let mut data = Vec::new();
        if self.database.is_text_type(db_tp) {
            // Text variables in the stack hold a Rust String (not a store record index).
            let store = self.database.store(&val);
            let s: &String = store.addr::<String>(val.rec, val.pos);
            data.extend_from_slice(s.as_bytes());
        } else if let Parts::Vector(elem_tp) = &self.database.types[db_tp as usize].parts {
            let elem_tp = *elem_tp;
            // Vector: `val` is a stack pointer holding a DbRef to the vector.
            // Dereference to reach the actual vector data, then write each element.
            let vec_ref = *self.database.store(&val).addr::<DbRef>(val.rec, val.pos);
            let (v_ptr, store_nr) = {
                let store = self.database.store(&vec_ref);
                (
                    store.get_int(vec_ref.rec, vec_ref.pos) as u32,
                    vec_ref.store_nr,
                )
            };
            if v_ptr != 0 {
                let length = self.database.allocations[store_nr as usize].get_int(v_ptr, 4) as u32;
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
        let written = data.len();
        if let Some(f) = &mut self.database.files[file_ref as usize]
            && let Err(e) = f.write_all(&data)
        {
            eprintln!("file write error: {e}");
        }
        // Update #next to reflect the end of this write.
        self.database
            .store_mut(&file)
            .set_long(file.rec, file.pos + 16, next_pos + written as i64);
    }

    pub fn read_file(&mut self) {
        let bytes = *self.get_stack::<i32>();
        let val = *self.get_stack::<DbRef>();
        let file = *self.get_stack::<DbRef>();
        let db_tp = *self.code::<u16>();
        if file.rec == 0 {
            return;
        }
        let f_nr = self.database.files.len() as i32;
        let store = self.database.store_mut(&file);
        let format = store.get_byte(file.rec, file.pos + 32, 0);
        // format: 1=TextFile, 2=LittleEndian, 3=BigEndian, 5=NotExists (default to TextFile).
        if format != 1 && format != 5 && format != 2 && format != 3 {
            return;
        }
        let little_endian = format == 2;
        let mut file_ref = store.get_int(file.rec, file.pos + 28);
        if file_ref == i32::MIN {
            let file_name = store.get_str(store.get_int(file.rec, file.pos + 24) as u32);
            if let Ok(f) = File::open(file_name) {
                store.set_int(file.rec, file.pos + 28, f_nr);
                self.database.files.push(Some(f));
            }
            file_ref = f_nr;
        }
        // Save the current position to the current field (file.current = old file.next)
        // Treat null (i64::MIN) as 0 (start of the file).
        let raw_next = self.database.store(&file).get_long(file.rec, file.pos + 16);
        let next_pos = if raw_next == i64::MIN { 0 } else { raw_next };
        self.database
            .store_mut(&file)
            .set_long(file.rec, file.pos + 8, next_pos);
        // Read the bytes
        let n = bytes as usize;
        let is_text = self.database.is_text_type(db_tp);
        let mut data = vec![0u8; n];
        let actual = if let Some(f) = &mut self.database.files[file_ref as usize] {
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
        // Update the next field with actual bytes read
        self.database
            .store_mut(&file)
            .set_long(file.rec, file.pos + 16, next_pos + actual as i64);
        if is_text {
            data.truncate(actual);
            // Text variables in the stack hold a Rust String; write directly into it.
            let s = unsafe { String::from_utf8_unchecked(data) };
            *self
                .database
                .store_mut(&val)
                .addr_mut::<String>(val.rec, val.pos) = s;
        } else if actual == n {
            self.database.write_data(&val, db_tp, little_endian, &data);
        }
        // For typed non-text reads with incomplete data: leave destination at null (already initialized)
    }

    pub fn seek_file(&mut self) {
        let pos = *self.get_stack::<i64>();
        let file = *self.get_stack::<DbRef>();
        if file.rec == 0 {
            return;
        }
        let file_ref = self.database.store(&file).get_int(file.rec, file.pos + 28);
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

    pub fn size_file(&mut self) {
        let file = *self.get_stack::<DbRef>();
        if file.rec == 0 {
            self.put_stack(i64::MIN);
            return;
        }
        let store = self.database.store(&file);
        let file_path = store
            .get_str(store.get_int(file.rec, file.pos + 24) as u32)
            .to_owned();
        let size = if let Ok(meta) = std::fs::metadata(&file_path) {
            meta.len() as i64
        } else {
            i64::MIN
        };
        self.put_stack(size);
    }

    pub fn truncate_file(&mut self) {
        let size = *self.get_stack::<i64>();
        let file = *self.get_stack::<DbRef>();
        if file.rec == 0 {
            self.put_stack(false);
            return;
        }
        let path = {
            let store = self.database.store(&file);
            store
                .get_str(store.get_int(file.rec, file.pos + 24) as u32)
                .to_owned()
        };
        // Close any open handle: the handle may be in read or write mode with a stale
        // position, and after resize the position might be beyond the new end of file.
        let file_ref = self.database.store(&file).get_int(file.rec, file.pos + 28);
        if file_ref != i32::MIN && (file_ref as usize) < self.database.files.len() {
            self.database.files[file_ref as usize] = None;
            self.database
                .store_mut(&file)
                .set_int(file.rec, file.pos + 28, i32::MIN);
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

    pub fn free_ref(&mut self) {
        let db = *self.get_stack::<DbRef>();
        if db.rec != 0
            && let Some(&file_type) = self.database.names.get("File")
        {
            let stored_type = self.database.store(&db).get_int(db.rec, 4) as u16;
            if stored_type == file_type {
                let file_ref = self.database.store(&db).get_int(db.rec, db.pos + 28);
                if file_ref != i32::MIN && (file_ref as usize) < self.database.files.len() {
                    self.database.files[file_ref as usize] = None;
                }
            }
        }
        self.database.free(&db);
    }

    pub fn format_database(&mut self) {
        let pos = *self.code::<u16>();
        let s = self.format_db();
        self.string_mut(pos - size_ref() as u16).push_str(&s);
    }

    pub fn format_stack_database(&mut self) {
        let pos = *self.code::<u16>();
        let s = self.format_db();
        self.string_ref_mut(pos - size_ref() as u16).push_str(&s);
    }

    pub fn sizeof_ref(&mut self) {
        let db = *self.get_stack::<DbRef>();
        let new_value = if db.rec == 0 {
            0i32
        } else {
            let db_tp = self.database.store(&db).get_int(db.rec, 4) as u16;
            i32::from(self.database.size(db_tp))
        };
        self.put_stack(new_value);
    }

    pub(super) fn format_db(&mut self) -> String {
        let db_tp = *self.code::<u16>();
        let format = *self.code::<u8>();
        let val = *self.get_stack::<DbRef>();
        let mut s = String::new();
        ShowDb {
            stores: &self.database,
            store: val.store_nr,
            rec: val.rec,
            pos: val.pos,
            known_type: db_tp,
            pretty: format & 1 > 0,
            json: format & 2 > 0,
        }
        .write(&mut s, 0);
        s
    }

    pub fn database(&mut self) {
        let var = *self.code::<u16>();
        let db_tp = *self.code::<u16>();
        let size = self.database.size(db_tp);
        let db = *self.get_var::<DbRef>(var);
        self.database.clear(&db);
        let r = self.database.claim(&db, u32::from(size));
        self.database
            .store_mut(&r)
            .set_int(r.rec, 4, i32::from(db_tp));
        self.database.set_default_value(db_tp, &r);
        let db = self.mut_var::<DbRef>(var);
        db.store_nr = r.store_nr;
        db.rec = 1;
        db.pos = 8;
    }

    pub fn new_record(&mut self) {
        let parent_tp = *self.code::<u16>();
        let fld = *self.code::<u16>();
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
    pub fn iterate(&mut self) {
        let on = *self.code::<u8>();
        let arg = *self.code::<u16>();
        let keys_size = *self.code::<u8>();
        let mut keys = Vec::new();
        for _ in 0..keys_size {
            keys.push(Key {
                type_nr: *self.code::<i8>(),
                position: *self.code::<u16>(),
            });
        }
        let from_key = *self.code::<u8>();
        let till_key = *self.code::<u8>();
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
        match on & 63 {
            1 => {
                // index points to the record position inside the store
                if reverse {
                    let t = tree::find(&data, ex, arg, all, &keys, &till);
                    start = if ex {
                        t
                    } else {
                        tree::next(crate::keys::store(&data, all), &new_ref(&data, t, arg))
                    };
                    let f = tree::find(&data, ex, arg, all, &keys, &from);
                    finish = tree::next(crate::keys::store(&data, all), &new_ref(&data, f, arg));
                } else {
                    start = tree::find(&data, true, arg, all, &keys, &from);
                    let t = tree::find(&data, ex, arg, all, &keys, &till);
                    finish = if ex {
                        t
                    } else {
                        tree::previous(crate::keys::store(&data, all), &new_ref(&data, t, arg))
                    };
                }
            }
            2 => {
                // sorted points to the position of the record inside the vector
                if reverse {
                    start =
                        vector::sorted_find(&data, ex, arg, all, &keys, &till).0 + u32::from(!ex);
                    finish = vector::sorted_find(&data, ex, arg, all, &keys, &from).0 + 1;
                } else {
                    let s = vector::sorted_find(&data, true, arg, all, &keys, &from).0;
                    start = if s == 0 { u32::MAX } else { s - 1 };
                    let (t, cmp) = vector::sorted_find(&data, ex, arg, all, &keys, &till);
                    finish = if ex || cmp { t } else { t + 1 };
                }
            }
            3 => {
                // ordered points to the position inside the vector of references
                if reverse {
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
        self.put_stack(start);
        self.put_stack(finish);
    }

    pub(super) fn stack_key(&mut self, size: u8, keys: &[Key]) -> Vec<Content> {
        let mut key = Vec::new();
        for (k_nr, k) in keys.iter().enumerate() {
            if k_nr >= size as usize {
                break;
            }
            match k.type_nr.abs() {
                1 => key.push(Content::Long(i64::from(*self.get_stack::<i32>()))),
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
        let state_var = *self.code::<u16>();
        let on = *self.code::<u8>();
        let arg = *self.code::<u16>();
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
                    let vector = store.get_int(data.rec, data.pos) as u32;
                    let rec = if pos == i32::MAX {
                        0
                    } else {
                        store.get_int(vector, pos as u32) as u32
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
        let state_var = *self.code::<u16>();
        let on = *self.code::<u8>();
        let tp = *self.code::<u16>();
        let reverse = on & 64 != 0;
        let cur = *self.get_var::<i32>(state_var);
        let data = *self.get_stack::<DbRef>();
        match on & 63 {
            0 => {
                // vector
                let n = if reverse { cur + 1 } else { cur - 1 };
                vector::remove_vector(
                    &data,
                    u32::from(self.database.size(tp)),
                    cur,
                    &mut self.database.allocations,
                );
                self.put_var(state_var - 8, n);
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
                    let finish = *self.get_var::<u32>(state_var - 16);
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
                vector::remove_vector(&data, u32::from(tp), cur, &mut self.database.allocations);
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
                    (cur - 8) / i32::from(tp),
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
        let tp = *self.code::<u16>();
        let data = *self.get_stack::<DbRef>();
        self.database.remove_claims(&data, tp);
    }

    pub fn append_copy(&mut self) {
        let tp = *self.code::<u16>();
        let multiply = *self.get_stack::<i32>() as u32;
        let data = *self.get_stack::<DbRef>();
        let ctp = self.database.content(tp);
        let size = u32::from(self.database.size(ctp));
        let length = vector::length_vector(&data, &self.database.allocations);
        let v_rec = crate::keys::store(&data, &self.database.allocations)
            .get_int(data.rec, data.pos) as u32;
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
        }
    }

    pub fn copy_record(&mut self) {
        let tp = *self.code::<u16>();
        let to = *self.get_stack::<DbRef>();
        let data = *self.get_stack::<DbRef>();
        let size = u32::from(self.database.size(tp));
        self.database.copy_block(&data, &to, size);
        self.database.copy_claims(&data, &to, tp);
    }

    pub fn hash_add(&mut self) {
        let tp = *self.code::<u16>();
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
        let tp = *self.code::<u16>();
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
        let tp = *self.code::<u16>();
        let rec = *self.get_stack::<DbRef>();
        let data = *self.get_stack::<DbRef>();
        if rec.rec != 0 {
            self.database.remove(&data, &rec, tp);
        }
    }

    pub(super) fn read_key(&mut self, full: bool) -> (u16, Vec<Content>) {
        let db_tp = *self.code::<u16>();
        let keys = self.database.get_keys(db_tp);
        let no_keys = if full {
            keys.len() as u8
        } else {
            *self.code::<u8>()
        };
        let mut key = Vec::new();
        for (k_nr, k) in keys.iter().enumerate() {
            if k_nr >= no_keys as usize {
                break;
            }
            match k {
                0 | 6 => key.push(Content::Long(i64::from(*self.get_stack::<i32>()))),
                1 => key.push(Content::Long(*self.get_stack::<i64>())),
                2 => key.push(Content::Single(*self.get_stack::<f32>())),
                3 => key.push(Content::Float(*self.get_stack::<f64>())),
                4 => key.push(Content::Long(i64::from(*self.get_stack::<bool>()))),
                5 => key.push(Content::Str(self.string())),
                _ => key.push(Content::Long(i64::from(*self.get_stack::<u8>()))),
            }
            // We assume that all none-base types are enumerate types.
        }
        (db_tp, key)
    }

    pub fn finish_record(&mut self) {
        let parent_tp = *self.code::<u16>();
        let fld = *self.code::<u16>();
        let record = *self.get_stack::<DbRef>();
        let data = *self.get_stack::<DbRef>();
        self.database.record_finish(&data, &record, parent_tp, fld);
    }

    pub fn db_from_text(&mut self, val: &str, db_tp: u16) -> DbRef {
        let db = self.database.database(8);
        let into = DbRef {
            store_nr: db.store_nr,
            rec: db.rec,
            pos: 8,
        };
        self.database.set_default_value(db_tp, &into);
        let mut pos = 0;
        // prevent throwing an error here
        if self
            .database
            .parsing(val, &mut pos, db_tp, db_tp, u16::MAX, &into)
        {
            into
        } else {
            DbRef {
                store_nr: db.store_nr,
                rec: 0,
                pos: 0,
            }
        }
    }

    pub fn insert_vector(&mut self) {
        let size = *self.code::<u16>();
        let db_tp = *self.code::<u16>();
        let index = *self.get_stack::<i32>();
        let r = *self.get_stack::<DbRef>();
        let new_value =
            vector::insert_vector(&r, u32::from(size), index, &mut self.database.allocations);
        self.database.set_default_value(db_tp, &new_value);
        self.put_stack(new_value);
    }
}
