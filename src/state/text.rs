// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use super::State;
use super::{size_ptr, size_str};
use crate::keys::{DbRef, Str};
use crate::ops;
use std::cmp::Ordering;

impl State {
    pub fn conv_text_from_null(&mut self) {
        self.put_stack(Str::new(super::STRING_NULL));
    }

    pub fn string_from_code(&mut self) {
        let size = *self.code::<u8>();
        unsafe {
            self.set_string(
                i32::from(size),
                self.bytecode.as_ptr().offset(self.code_pos as isize),
            );
        }
        self.code_pos += u32::from(size);
    }

    pub(super) unsafe fn set_string(&mut self, size: i32, off: *const u8) {
        let m = self
            .database
            .store_mut(&self.stack_cur)
            .addr_mut::<Str>(self.stack_cur.rec, self.stack_cur.pos + self.stack_pos);
        *m = Str {
            ptr: off,
            len: size as u32,
        };
        self.stack_pos += size_of::<Str>() as u32;
    }

    pub fn string_from_texts(&mut self, start: i32, size: i32) {
        unsafe {
            self.set_string(size, self.text_code.as_ptr().offset(start as isize));
        }
    }

    #[must_use]
    pub fn string(&mut self) -> Str {
        self.stack_pos -= size_ptr();
        *self
            .database
            .store(&self.stack_cur)
            .addr::<Str>(self.stack_cur.rec, self.stack_cur.pos + self.stack_pos)
    }

    #[inline]
    pub fn length_character(&mut self) {
        let v_v1 = *self.get_stack::<char>();
        let new_value = if v_v1 == char::from(0) {
            0
        } else {
            v_v1.to_string().len() as i32
        };
        self.put_stack(new_value);
    }

    #[inline]
    pub fn append_text(&mut self) {
        let text = self.string();
        let pos = *self.code::<u16>();
        if cfg!(debug_assertions) {
            self.text_positions
                .insert(self.stack_cur.pos + self.stack_pos + size_ptr() - u32::from(pos));
        }
        let v1 = self.string_mut(pos - size_ptr() as u16);
        *v1 += text.str();
    }

    /// `OpCreateStack`: push a `DbRef` pointing into the current stack frame.
    /// Used as null-state for borrowed references; must be overwritten by `OpPutRef`
    /// before any field access.
    #[inline]
    pub fn create_stack(&mut self) {
        let pos = *self.code::<u16>();
        let db = DbRef {
            store_nr: self.stack_cur.store_nr,
            rec: self.stack_cur.rec,
            pos: self.stack_cur.pos + self.stack_pos - u32::from(pos),
        };
        self.put_stack(db);
    }

    #[inline]
    pub fn get_stack_text(&mut self) {
        let r = *self.get_stack::<DbRef>();
        let t: &str = self.database.store(&r).addr::<String>(r.rec, r.pos);
        self.put_stack(Str::new(t));
    }

    #[inline]
    pub fn get_stack_ref(&mut self) {
        let fld = *self.code::<u16>();
        let r = *self.get_stack::<DbRef>();
        let t = self
            .database
            .store(&r)
            .addr::<DbRef>(r.rec, r.pos + u32::from(fld));
        self.put_stack(*t);
    }

    #[inline]
    pub fn set_stack_ref(&mut self) {
        let v1 = *self.get_stack::<DbRef>();
        let r = *self.get_stack::<DbRef>();
        let t = self.database.store_mut(&r).addr_mut::<DbRef>(r.rec, r.pos);
        *t = v1;
    }

    pub fn append_stack_text(&mut self) {
        let text = self.string();
        let pos = *self.code::<u16>();
        let v1 = self.string_ref_mut(pos - size_ptr() as u16);
        *v1 += text.str();
    }

    pub fn append_stack_character(&mut self) {
        let pos = *self.code::<u16>();
        let c = *self.get_stack::<char>();
        if c as u32 != 0 {
            self.string_ref_mut(pos - 4).push(c);
        }
    }

    pub fn clear_stack_text(&mut self) {
        let pos = *self.code::<u16>();
        let v1 = self.string_ref_mut(pos);
        v1.clear();
    }

    #[inline]
    pub fn append_character(&mut self) {
        let pos = *self.code::<u16>();
        let c = *self.get_stack::<char>();
        if c as u32 != 0 {
            self.string_mut(pos - 4).push(c);
        }
    }

    #[inline]
    pub fn text_compare(&mut self) {
        let v2 = *self.get_stack::<char>();
        let v1 = *self.get_stack::<Str>();
        let mut ch = v1.str().chars();
        self.put_stack(if let Some(f_ch) = ch.next() {
            let res = f_ch.cmp(&v2);
            if res == Ordering::Less {
                -1
            } else {
                i32::from(
                    res == Ordering::Greater || (res == Ordering::Equal && ch.next().is_some()),
                )
            }
        } else {
            -1
        });
    }

    #[must_use]
    pub fn lines_text<'b>(val: &'b str, at: &mut i32) -> &'b str {
        if let Some(to) = val[*at as usize..].find('\n') {
            let r = &val[*at as usize..to];
            *at = to as i32 + 1;
            r
        } else {
            *at = i32::MIN;
            ""
        }
    }

    #[must_use]
    pub fn split_text<'b>(val: &'b str, on: &str, at: &mut i32) -> &'b str {
        if on.is_empty() {
            *at = i32::MIN;
            return "";
        }
        if let Some(to) = val[*at as usize..].find(on) {
            let r = &val[*at as usize..to];
            *at = (to + on.len()) as i32;
            r
        } else {
            *at = i32::MIN;
            ""
        }
    }

    #[inline]
    pub fn get_text_sub(&mut self) {
        let mut till = *self.get_stack::<i32>();
        let mut from = *self.get_stack::<i32>();
        let v1 = self.string();
        if from < 0 || from >= v1.len as i32 {
            self.put_stack(Str {
                ptr: v1.ptr,
                len: 0,
            });
            return;
        }
        let mut b = v1.str().as_bytes()[from as usize];
        while b & 0xC0 == 0x80 && from > 0 {
            from -= 1;
            b = v1.str().as_bytes()[from as usize];
        }
        if till == i32::MIN {
            let b = v1.str().as_bytes()[from as usize];
            let ch = if b & 0xE0 == 0xC0 {
                from as u32 + 2
            } else if b & 0xF0 == 0xE0 {
                from as u32 + 3
            } else if b & 0xF0 == 0xF0 {
                from as u32 + 4
            } else {
                from as u32 + 1
            };
            let res = unsafe {
                Str {
                    ptr: v1.ptr.offset(from as isize),
                    len: ch - from as u32,
                }
            };
            self.put_stack(res);
            return;
        }
        if till < 0 {
            till += v1.len as i32;
        }
        let mut len = till - from;
        if len <= 0 {
            self.put_stack(Str {
                ptr: v1.ptr,
                len: 0,
            });
            return;
        }
        if len + from > v1.len as i32 {
            len = v1.len as i32 - from;
        } else if till < v1.len as i32 {
            let mut t = till;
            let mut b = v1.str().as_bytes()[t as usize];
            while b & 0xC0 == 0x80 && t < v1.len as i32 {
                t += 1;
                b = v1.str().as_bytes()[t as usize];
                len += 1;
            }
        }
        unsafe {
            self.put_stack(Str {
                ptr: v1.ptr.offset(from as isize),
                len: len as u32,
            });
        }
    }

    pub fn clear_text(&mut self) {
        let pos = *self.code::<u16>();
        self.string_mut(pos).clear();
    }

    /**
    Free the content of a text variable.
    # Panics
    When the same variable is freed twice.
    */
    pub fn free_text(&mut self) {
        let pos = *self.code::<u16>();
        if cfg!(debug_assertions) {
            let s = self.string_mut(pos);
            s.clear();
            for _ in 0..s.len() {
                *s += "*";
            }
        }
        self.string_mut(pos).shrink_to(0);
        if cfg!(debug_assertions) {
            let var_pos = self.stack_cur.pos + self.stack_pos - u32::from(pos);
            let remove = self.text_positions.remove(&var_pos);
            assert!(remove, "double free");
        }
    }

    /** Get a string reference from a variety of internal string formats.
    # Panics
    When an unknown internal string format is found.
    */
    pub fn string_mut(&mut self, pos: u16) -> &mut String {
        self.database.store_mut(&self.stack_cur).addr_mut::<String>(
            self.stack_cur.rec,
            self.stack_cur.pos + self.stack_pos - u32::from(pos),
        )
    }

    pub(super) fn string_ref_mut(&mut self, pos: u16) -> &mut String {
        let r = *self.database.store(&self.stack_cur).addr::<DbRef>(
            self.stack_cur.rec,
            self.stack_cur.pos + self.stack_pos - u32::from(pos),
        );
        self.database
            .store_mut(&self.stack_cur)
            .addr_mut::<String>(r.rec, r.pos)
    }

    pub fn text(&mut self) {
        if cfg!(debug_assertions) {
            self.text_positions
                .insert(self.stack_cur.pos + self.stack_pos);
        }
        let v = self.string_mut(0);
        let s = String::new();
        unsafe {
            core::ptr::write(v, s);
        }
        self.stack_pos += size_str();
    }

    pub fn var_text(&mut self) {
        let pos = *self.code::<u16>();
        let new_value = Str::new(self.get_var::<String>(pos));
        self.put_stack(new_value);
    }

    pub fn arg_text(&mut self) {
        let pos = *self.code::<u16>();
        let new_value = *self.get_var::<Str>(pos);
        self.put_stack(new_value);
    }

    pub fn format_long(&mut self) {
        let pos = *self.code::<u16>();
        let radix = *self.code::<u8>();
        let token = *self.code::<u8>();
        let plus = *self.code::<bool>();
        let note = *self.code::<bool>();
        let width = *self.get_stack::<i32>();
        let val = *self.get_stack::<i64>();
        let s = self.string_mut(pos - 12);
        ops::format_long(s, val, radix, width, token, plus, note);
    }

    pub fn format_stack_long(&mut self) {
        let pos = *self.code::<u16>();
        let radix = *self.code::<u8>();
        let token = *self.code::<u8>();
        let plus = *self.code::<bool>();
        let note = *self.code::<bool>();
        let width = *self.get_stack::<i32>();
        let val = *self.get_stack::<i64>();
        let s = self.string_ref_mut(pos - 12);
        ops::format_long(s, val, radix, width, token, plus, note);
    }

    pub fn format_float(&mut self) {
        let pos = *self.code::<u16>();
        let precision = *self.get_stack::<i32>();
        let width = *self.get_stack::<i32>();
        let val = *self.get_stack::<f64>();
        let s = self.string_mut(pos - 16);
        ops::format_float(s, val, width, precision);
    }

    pub fn format_stack_float(&mut self) {
        let pos = *self.code::<u16>();
        let precision = *self.get_stack::<i32>();
        let width = *self.get_stack::<i32>();
        let val = *self.get_stack::<f64>();
        let s = self.string_ref_mut(pos - 16); // f64(8)+i32(4)+i32(4) = 16 bytes popped
        ops::format_float(s, val, width, precision);
    }

    pub fn format_single(&mut self) {
        let pos = *self.code::<u16>();
        let precision = *self.get_stack::<i32>();
        let width = *self.get_stack::<i32>();
        let val = *self.get_stack::<f32>();
        let s = self.string_mut(pos - 12);
        ops::format_single(s, val, width, precision);
    }

    pub fn format_stack_single(&mut self) {
        let pos = *self.code::<u16>();
        let precision = *self.get_stack::<i32>();
        let width = *self.get_stack::<i32>();
        let val = *self.get_stack::<f32>();
        let s = self.string_ref_mut(pos - 12);
        ops::format_single(s, val, width, precision);
    }

    pub fn format_text(&mut self) {
        let pos = *self.code::<u16>();
        let dir = *self.code::<i8>();
        let token = *self.code::<u8>();
        let width = *self.get_stack::<i32>();
        let val = self.string();
        let s = self.string_mut(pos - 4 - size_ptr() as u16);
        ops::format_text(s, val.str(), width, dir, token);
    }

    pub fn format_stack_text(&mut self) {
        let pos = *self.code::<u16>();
        let dir = *self.code::<i8>();
        let token = *self.code::<u8>();
        let width = *self.get_stack::<i32>();
        let val = self.string();
        let s = self.string_ref_mut(pos - 4 - size_ptr() as u16);
        ops::format_text(s, val.str(), width, dir, token);
    }
}
