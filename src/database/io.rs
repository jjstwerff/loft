// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! File I/O: reading and writing database content to/from files.

use crate::database::{Parts, Stores};
use crate::keys::DbRef;
use crate::store::Store;
use crate::vector;
use std::collections::BTreeMap;
use std::io::Write as _;

enum Format {
    TextFile = 1,
    _LittleEndian = 2,
    _BigEndian = 3,
    Directory = 4,
    NotExists = 5,
}

fn fill_file(path: &std::path::Path, store: &mut Store, file: &DbRef) -> bool {
    store.set_long(file.rec, file.pos + 8, i64::MIN); // current
    store.set_long(file.rec, file.pos + 16, i64::MIN); // next
    if let Ok(data) = path.metadata() {
        store.set_long(file.rec, file.pos, i64::MIN); // no size
        let tp = if data.is_dir() {
            Format::Directory
        } else if data.is_file() {
            store.set_long(file.rec, file.pos, data.len() as i64); // write size
            Format::TextFile
        } else {
            Format::NotExists
        };
        store.set_byte(file.rec, file.pos + 32, 0, tp as i32);
        true
    } else {
        store.set_byte(file.rec, file.pos + 32, 0, Format::NotExists as i32);
        false
    }
}

impl Stores {
    /// # Panics
    /// If `tp` refers to a type that is not implemented for file reading.
    #[allow(clippy::too_many_lines)]
    pub fn read_data(&self, r: &DbRef, tp: u16, little_endian: bool, data: &mut Vec<u8>) {
        let store = &self.allocations[r.store_nr as usize];
        match tp {
            0 | 6 => {
                // integer | character
                let v = store.get_int(r.rec, r.pos);
                (if little_endian {
                    v.to_le_bytes()
                } else {
                    v.to_be_bytes()
                })
                .iter()
                .for_each(|&x| data.push(x));
            }
            1 => {
                // long
                let v = store.get_long(r.rec, r.pos);
                (if little_endian {
                    v.to_le_bytes()
                } else {
                    v.to_be_bytes()
                })
                .iter()
                .for_each(|&x| data.push(x));
            }
            2 => {
                // single
                let v = store.get_single(r.rec, r.pos);
                (if little_endian {
                    v.to_le_bytes()
                } else {
                    v.to_be_bytes()
                })
                .iter()
                .for_each(|&x| data.push(x));
            }
            3 => {
                // float
                let v = store.get_float(r.rec, r.pos);
                (if little_endian {
                    v.to_le_bytes()
                } else {
                    v.to_be_bytes()
                })
                .iter()
                .for_each(|&x| data.push(x));
            }
            4 => {
                // boolean
                let v = store.get_byte(r.rec, r.pos, 0) as u8;
                data.push(v);
            }
            5 => {
                // text
                let v = store.get_str(store.get_int(r.rec, r.pos) as u32);
                v.as_bytes().iter().for_each(|&x| data.push(x));
            }
            _ => match self.types[tp as usize].parts.clone() {
                Parts::Struct(s) | Parts::EnumValue(_, s) => {
                    for f in &s {
                        if f.name == "enum" || f.position == u16::MAX {
                            continue;
                        }
                        let field_r = DbRef {
                            store_nr: r.store_nr,
                            rec: r.rec,
                            pos: r.pos + u32::from(f.position),
                        };
                        self.read_data(&field_r, f.content, little_endian, data);
                    }
                }
                Parts::Enum(_) => {
                    data.push(store.get_byte(r.rec, r.pos, 0) as u8);
                }
                Parts::Byte(_, _) => {
                    data.push(store.get_int(r.rec, r.pos) as u8);
                }
                Parts::Short(_, _) => {
                    let v = store.get_int(r.rec, r.pos) as i16;
                    (if little_endian {
                        v.to_le_bytes()
                    } else {
                        v.to_be_bytes()
                    })
                    .iter()
                    .for_each(|&x| data.push(x));
                }
                Parts::Vector(elem_tp) => {
                    let v_rec = {
                        let store = &self.allocations[r.store_nr as usize];
                        store.get_int(r.rec, r.pos) as u32
                    };
                    let length = if v_rec == 0 {
                        0u32
                    } else {
                        let store = &self.allocations[r.store_nr as usize];
                        store.get_int(v_rec, 4) as u32
                    };
                    let elem_size = u32::from(self.size(elem_tp));
                    let store_nr = r.store_nr;
                    for i in 0..length {
                        let elem = DbRef {
                            store_nr,
                            rec: v_rec,
                            pos: 8 + elem_size * i,
                        };
                        self.read_data(&elem, elem_tp, little_endian, data);
                    }
                }
                Parts::Array(elem_tp) => {
                    let v_rec = {
                        let store = &self.allocations[r.store_nr as usize];
                        store.get_int(r.rec, r.pos) as u32
                    };
                    let length = if v_rec == 0 {
                        0u32
                    } else {
                        let store = &self.allocations[r.store_nr as usize];
                        store.get_int(v_rec, 4) as u32
                    };
                    let store_nr = r.store_nr;
                    for i in 0..length {
                        let elm_rec = {
                            let store = &self.allocations[store_nr as usize];
                            store.get_int(v_rec, 8 + 4 * i) as u32
                        };
                        let elem = DbRef {
                            store_nr,
                            rec: elm_rec,
                            pos: 8,
                        };
                        self.read_data(&elem, elem_tp, little_endian, data);
                    }
                }
                Parts::Sorted(_, _)
                | Parts::Ordered(_, _)
                | Parts::Hash(_, _)
                | Parts::Index(_, _, _)
                | Parts::Spacial(_, _) => panic!(
                    "binary I/O not supported for type '{}': it contains a collection field \
                     with store-internal references that cannot be serialized",
                    self.types[tp as usize].name
                ),
                Parts::Base => unreachable!(
                    "Parts::Base should never appear as a field type in read_data \
                     (type: {})",
                    self.types[tp as usize].name
                ),
            },
        }
    }

    /// Return the number of bytes that `read_data` will append for the given type.
    /// Returns 0 for types whose binary size is variable (text) or unsupported (collections).
    fn binary_size(&self, tp: u16) -> usize {
        match tp {
            0 | 2 | 6 => 4, // integer, single, character
            1 | 3 => 8,     // long, float
            4 => 1,         // boolean
            5 => 0,         // text: variable length
            _ => match &self.types[tp as usize].parts {
                Parts::Struct(s) | Parts::EnumValue(_, s) => s
                    .iter()
                    .filter(|f| f.name != "enum" && f.position != u16::MAX)
                    .map(|f| self.binary_size(f.content))
                    .sum(),
                Parts::Enum(_) | Parts::Byte(_, _) => 1,
                Parts::Short(_, _) => 2,
                _ => 0,
            },
        }
    }

    /// # Panics
    /// If `data` does not contain enough bytes for the given type.
    #[allow(clippy::too_many_lines)]
    pub fn write_data(&mut self, r: &DbRef, tp: u16, little_endian: bool, data: &[u8]) {
        let store = &mut self.allocations[r.store_nr as usize];
        match tp {
            0 | 6 => {
                let d = data[0..4].try_into().unwrap();
                let v = if little_endian {
                    i32::from_le_bytes(d)
                } else {
                    i32::from_be_bytes(d)
                };
                store.set_int(r.rec, r.pos, v);
            }
            1 => {
                // long
                let d = data[0..8].try_into().unwrap();
                let v = if little_endian {
                    i64::from_le_bytes(d)
                } else {
                    i64::from_be_bytes(d)
                };
                store.set_long(r.rec, r.pos, v);
            }
            2 => {
                // single
                let d = data[0..4].try_into().unwrap();
                let v = if little_endian {
                    f32::from_le_bytes(d)
                } else {
                    f32::from_be_bytes(d)
                };
                store.set_single(r.rec, r.pos, v);
            }
            3 => {
                // float
                let d = data[0..8].try_into().unwrap();
                let v = if little_endian {
                    f64::from_le_bytes(d)
                } else {
                    f64::from_be_bytes(d)
                };
                store.set_float(r.rec, r.pos, v);
            }
            4 => {
                // boolean
                let v = data[0];
                store.set_byte(r.rec, r.pos, 0, i32::from(v));
            }
            5 => {
                // text
                let v = unsafe {
                    let mut v = Vec::new();
                    v.extend_from_slice(data);
                    String::from_utf8_unchecked(v)
                };
                let s = store.set_str(v.as_str());
                store.set_int(r.rec, r.pos, s as i32);
            }
            _ => match self.types[tp as usize].parts.clone() {
                Parts::Struct(s) | Parts::EnumValue(_, s) => {
                    let mut offset = 0usize;
                    for f in &s {
                        if f.name == "enum" || f.position == u16::MAX {
                            continue;
                        }
                        let field_r = DbRef {
                            store_nr: r.store_nr,
                            rec: r.rec,
                            pos: r.pos + u32::from(f.position),
                        };
                        self.write_data(&field_r, f.content, little_endian, &data[offset..]);
                        offset += self.binary_size(f.content);
                    }
                }
                Parts::Enum(_) | Parts::Byte(_, _) => {
                    store.set_int(r.rec, r.pos, i32::from(data[0]));
                }
                Parts::Short(_, _) => {
                    let d: [u8; 2] = data[0..2].try_into().unwrap();
                    let v = if little_endian {
                        i32::from(i16::from_le_bytes(d))
                    } else {
                        i32::from(i16::from_be_bytes(d))
                    };
                    store.set_int(r.rec, r.pos, v);
                }
                Parts::Vector(elem_tp) => {
                    let elem_size = u32::from(self.size(elem_tp));
                    if elem_size == 0 {
                        return;
                    }
                    let n_elems = data.len() / elem_size as usize;
                    for i in 0..n_elems {
                        let elem_ref = vector::vector_append(r, elem_size, &mut self.allocations);
                        let slice = &data[i * elem_size as usize..(i + 1) * elem_size as usize];
                        self.write_data(&elem_ref, elem_tp, little_endian, slice);
                        vector::vector_finish(r, &mut self.allocations);
                    }
                }
                Parts::Array(_)
                | Parts::Sorted(_, _)
                | Parts::Ordered(_, _)
                | Parts::Hash(_, _)
                | Parts::Index(_, _, _)
                | Parts::Spacial(_, _) => panic!(
                    "binary I/O not supported for type '{}': it contains a collection field \
                     with store-internal references that cannot be serialized",
                    self.types[tp as usize].name
                ),
                Parts::Base => unreachable!(
                    "Parts::Base should never appear as a field type in write_data \
                     (type: {})",
                    self.types[tp as usize].name
                ),
            },
        }
    }

    pub fn get_file(&mut self, file: &DbRef) -> bool {
        if file.rec == 0 {
            return false;
        }
        let store = self.store_mut(file);
        let filename = store.get_str(store.get_int(file.rec, file.pos + 24) as u32);
        let path = std::path::Path::new(filename);
        fill_file(path, store, file)
    }

    pub fn get_dir(&mut self, file_path: &str, result: &DbRef) -> bool {
        let path = std::path::Path::new(&file_path);
        if let Ok(iter) = std::fs::read_dir(path) {
            let vector = DbRef {
                store_nr: result.store_nr,
                rec: result.rec,
                pos: result.pos,
            };
            let mut res = BTreeMap::new();
            for entry in iter.flatten() {
                if let Some(name) = entry.path().to_str() {
                    // Normalise to forward slashes so loft paths are consistent on
                    // all platforms (Windows returns backslash-separated paths).
                    res.insert(name.replace('\\', "/"), entry);
                } else {
                    return false;
                }
            }
            for (name, entry) in res {
                let elm = vector::vector_append(&vector, 33, &mut self.allocations);
                let store = self.store_mut(result);
                let name_pos = store.set_str(&name) as i32;
                store.set_int(elm.rec, elm.pos + 24, name_pos);
                store.set_int(elm.rec, elm.pos + 28, i32::MIN);
                // Initialize current and next to null (i64::MIN) so they're not shown
                store.set_long(elm.rec, elm.pos + 8, i64::MIN);
                store.set_long(elm.rec, elm.pos + 16, i64::MIN);
                vector::vector_finish(&vector, &mut self.allocations);
                let store = self.store_mut(result);
                if !fill_file(&entry.path(), store, &elm) {
                    return false;
                }
            }
        }
        true
    }

    /**
    Read the binary data from a png image.
    # Panics
    On file system problems
    */
    #[cfg(feature = "png")]
    pub fn get_png(&mut self, file_path: &str, result: &DbRef) -> bool {
        let store = self.store_mut(result);
        if let Ok((img, width, height)) = crate::png_store::read(file_path, store) {
            if let Some(name) = std::path::Path::new(&file_path).file_name() {
                let name_pos = store.set_str(name.to_str().unwrap());
                store.set_int(result.rec, result.pos + 4, name_pos as i32);
                store.set_int(result.rec, result.pos + 8, width as i32);
                store.set_int(result.rec, result.pos + 12, height as i32);
                store.set_int(result.rec, result.pos + 16, img as i32);
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    #[cfg(not(feature = "png"))]
    pub fn get_png(&mut self, _file_path: &str, _result: &DbRef) -> bool {
        false
    }

    pub fn write_file(&mut self, file: &DbRef, v: &str) {
        let f_nr = self.files.len() as i32;
        let s = self.store_mut(file);
        let mut file_ref = s.get_int(file.rec, file.pos + 28);
        if file_ref == i32::MIN {
            let file_name = s.get_str(s.get_int(file.rec, file.pos + 24) as u32);
            if let Ok(f) = std::fs::File::create(file_name) {
                s.set_int(file.rec, file.pos + 28, f_nr);
                self.files.push(Some(f));
            }
            file_ref = f_nr;
        }
        if let Some(f) = &mut self.files[file_ref as usize] {
            f.write_all(v.as_bytes()).unwrap_or_default();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// S4 — Issue 59: `read_data Parts::Struct` must use `r.pos + f.position` for every
    /// field, not `r.pos` for all fields.  Without the fix both fields serialise the same
    /// value (the first field is read twice).
    #[test]
    fn read_data_struct_field_positions() {
        let mut stores = Stores::new();
        // Define struct Pair { a: integer, b: integer }
        let pair_tp = stores.structure("Pair", -1);
        stores.field(pair_tp, "a", 0); // integer
        stores.field(pair_tp, "b", 0); // integer
        stores.finish();

        // Allocate a record: 3 words = 24 bytes
        //   word 0 byte 0..3: claim counter
        //   word 0 byte 4..7: (header padding / parent ptr for top-level)
        //   word 1 byte 0..3: field a  (pos 8 + struct-position 0 = 8)
        //   word 1 byte 4..7: field b  (pos 8 + struct-position 4 = 12)
        let db = stores.database(3);
        {
            let s = &mut stores.allocations[db.store_nr as usize];
            s.set_int(db.rec, db.pos, 10); // a = 10
            s.set_int(db.rec, db.pos + 4, 20); // b = 20
        }

        let mut data = Vec::new();
        stores.read_data(&db, pair_tp, true, &mut data);

        assert_eq!(data.len(), 8, "Pair binary size: 2 × i32 = 8 bytes");
        let a_val = i32::from_le_bytes(data[0..4].try_into().unwrap());
        let b_val = i32::from_le_bytes(data[4..8].try_into().unwrap());
        assert_eq!(a_val, 10, "field a should be 10");
        assert_eq!(b_val, 20, "field b should be 20 (was 10 before fix)");
    }

    /// S4 — Issue 59: `write_data Parts::Struct` must use `r.pos + f.position` for every
    /// field and advance the data-slice offset between fields.
    #[test]
    fn write_data_struct_field_positions() {
        let mut stores = Stores::new();
        let pair_tp = stores.structure("Pair", -1);
        stores.field(pair_tp, "a", 0);
        stores.field(pair_tp, "b", 0);
        stores.finish();

        let db = stores.database(3);
        // Write [a=77_le, b=99_le] into the struct
        let bytes: Vec<u8> = {
            let mut v = 77i32.to_le_bytes().to_vec();
            v.extend_from_slice(&99i32.to_le_bytes());
            v
        };
        stores.write_data(&db, pair_tp, true, &bytes);

        let s = &stores.allocations[db.store_nr as usize];
        let a_val = s.get_int(db.rec, db.pos);
        let b_val = s.get_int(db.rec, db.pos + 4);
        assert_eq!(a_val, 77, "field a should be 77");
        assert_eq!(b_val, 99, "field b should be 99 (was 77 before fix)");
    }
}
