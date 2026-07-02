// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I66 — Bytecode VM / executor

use super::State;
use super::size_ptr;
use crate::keys::{DbRef, Str};
use crate::ops;
use std::cmp::Ordering;

impl State {
    pub fn conv_text_from_null(&mut self) {
        self.put_stack(Str::new(super::STRING_NULL));
    }

    pub fn string_from_code(&mut self) {
        let size = self.code::<u8>();
        unsafe {
            self.set_string(
                i32::from(size),
                self.bytecode.as_ptr().offset(self.code_pos as isize),
            );
        }
        self.code_pos += u32::from(size);
    }

    pub(super) unsafe fn set_string(&mut self, size: i32, off: *const u8) {
        #[cfg(feature = "stack_align_guard")]
        self.check_stack_align::<Str>(self.stack_cur.pos + self.stack_pos);
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

    /// # Panics
    /// Always — `OpConstLongText` is obsolete; long strings now use `OpConstStoreText`.
    #[allow(clippy::unused_self)]
    pub fn string_from_texts(&mut self, _start: i64, _size: i64) {
        panic!("OpConstLongText is obsolete — use OpConstStoreText");
    }

    /// Push a Str pointing into the constant store's string data.
    /// The string was written by `Store::set_str()` during compilation:
    /// length at `get_int(rec, 4)`, bytes at `ptr + rec*8 + 8`.
    pub fn string_from_const_store(&mut self, rec: u32, _pos: u32) {
        let store = &self.database.allocations[crate::database::CONST_STORE as usize];
        let len = store.get_u32_raw(rec, 4) as i32;
        let ptr = unsafe { store.ptr.offset(rec as isize * 8 + 8) };
        unsafe {
            self.set_string(len, ptr);
        }
    }

    #[must_use]
    pub fn string(&mut self) -> Str {
        self.stack_pos -= size_ptr();
        #[cfg(feature = "stack_align_guard")]
        self.check_stack_align::<Str>(self.stack_cur.pos + self.stack_pos);
        *self
            .database
            .store(&self.stack_cur)
            .addr::<Str>(self.stack_cur.rec, self.stack_cur.pos + self.stack_pos)
    }

    #[inline]
    pub fn length_character(&mut self) {
        let v_v1 = *self.get_stack::<char>();
        let new_value: i64 = if v_v1 == char::from(0) {
            0
        } else {
            v_v1.to_string().len() as i64
        };
        self.put_stack(new_value);
    }

    #[inline]
    pub fn append_text(&mut self) {
        let text = self.string();
        let pos = self.code::<u16>();
        if cfg!(debug_assertions) {
            self.text_positions
                .insert(self.stack_cur.pos + self.stack_pos + size_ptr() - u32::from(pos));
        }
        let v1 = self.string_mut(pos - size_ptr() as u16);
        // @PLN25 slice (c): a null `text?` accumulator (the STRING_NULL sentinel) IGNORES the
        // append, so `s += x` on a null `s` stays null (propagate — nulls stay visible, never
        // swept into "\0x"). A non-null text is never the sentinel, so this never fires for it.
        // DN1-gated so gate-OFF (the legacy nullable model, where a plain text can still be
        // STRING_NULL) keeps its byte-identical append behaviour.
        if crate::keys::pln25_dn1_enabled() && v1.as_str() == super::STRING_NULL {
            return;
        }
        *v1 += text.str();
    }

    /// `OpCreateStack`: push a `DbRef` pointing into the current stack frame.
    /// Used as null-state for borrowed references; must be overwritten by `OpPutRef`
    /// before any field access.
    #[inline]
    pub fn create_stack(&mut self) {
        let pos = self.code::<u16>();
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
        let fld = self.code::<u16>();
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
        let pos = self.code::<u16>();
        let v1 = self.string_ref_mut(pos - size_ptr() as u16);
        *v1 += text.str();
    }

    pub fn append_stack_character(&mut self) {
        let pos = self.code::<u16>();
        let c = *self.get_stack::<char>();
        if c as u32 != 0 {
            // @PLAN53 cluster 2 / S4: the char pop occupies a stepped span
            // (4 off, 8 aligned) — N = bytes the op's get_stack popped.
            let off = pos - self.stack_step(4) as u16;
            self.string_ref_mut(off).push(c);
        }
    }

    pub fn clear_stack_text(&mut self) {
        let pos = self.code::<u16>();
        let v1 = self.string_ref_mut(pos);
        v1.clear();
    }

    #[inline]
    pub fn append_character(&mut self) {
        let pos = self.code::<u16>();
        let c = *self.get_stack::<char>();
        // @PLAN53 cluster 2 / S4: stepped char-pop span (4 off, 8 aligned).
        let n = self.stack_step(4) as u16;
        // Plan-07 phase 4e.3 — when a fault tag is set on the
        // preceding `OpTagFault` (4e.1 format-scope swap) AND the
        // char is the null sentinel, render `null(<tag>)` so the
        // developer sees what produced the missing character
        // instead of an empty space.  Bare `'\0'` outside format
        // scope (no tag) keeps the legacy "skip" behaviour so
        // string concatenation with literal `'\0'` is unchanged.
        let tag = self.database.take_format_fault();
        if c as u32 == 0 {
            if let Some(label) = tag {
                let s = self.string_mut(pos - n);
                s.push_str("null(");
                s.push_str(label);
                s.push(')');
            }
            return;
        }
        self.string_mut(pos - n).push(c);
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
        let mut till = *self.get_stack::<i64>() as i32;
        let mut from = *self.get_stack::<i64>() as i32;
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
        let pos = self.code::<u16>();
        self.string_mut(pos).clear();
    }

    /**
    Free the content of a text variable.
    # Panics
    When the same variable is freed twice.
    */
    pub fn free_text(&mut self) {
        let pos = self.code::<u16>();
        // @PLAN53 cluster 5: deallocate the String's heap buffer.  `clear()` sets
        // len=0 so the following `shrink_to(0)` drops capacity to 0 and frees the
        // buffer.  Previously `clear()` ran ONLY under debug-assertions, so in
        // release — and under Miri, where the lib disables debug-assertions — a
        // non-empty String reached `shrink_to(0)`, which shrinks capacity only down
        // to `len` (never below), leaving the buffer allocated and LEAKED (the store
        // holds the String as raw bytes and never runs its Drop; `free_text` is the
        // sole deallocation point).  Miri's leak checker caught it (cluster 5).
        // (The old debug-only `for _ in 0..s.len()` poison loop was dead code — it
        // ran after `clear()`, so `s.len()` was already 0 and it never executed.)
        let s = self.string_mut(pos);
        s.clear();
        s.shrink_to(0);
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
        #[cfg(feature = "stack_align_guard")]
        self.check_stack_align::<String>(self.stack_cur.pos + self.stack_pos - u32::from(pos));
        self.database.store_mut(&self.stack_cur).addr_mut::<String>(
            self.stack_cur.rec,
            self.stack_cur.pos + self.stack_pos - u32::from(pos),
        )
    }

    pub(super) fn string_ref_mut(&mut self, pos: u16) -> &mut String {
        #[cfg(feature = "stack_align_guard")]
        self.check_stack_align::<DbRef>(self.stack_cur.pos + self.stack_pos - u32::from(pos));
        let r = *self.database.store(&self.stack_cur).addr::<DbRef>(
            self.stack_cur.rec,
            self.stack_cur.pos + self.stack_pos - u32::from(pos),
        );
        self.database
            .store_mut(&self.stack_cur)
            .addr_mut::<String>(r.rec, r.pos)
    }

    /// Plan-04 Phase 2h: positional variant of the removed `text()`.  Writes a
    /// fresh empty `String` at the frame slot reached by
    /// `self.stack_pos - pos` (matches the addressing convention of
    /// `clear_text`, `append_text`, and the other `_text` ops).
    /// Does NOT advance `stack_pos` — the frame is assumed to be
    /// already reserved.
    ///
    /// `pos` is read from the bytecode stream as a `const u16`.
    pub fn init_text(&mut self) {
        let pos = self.code::<u16>();
        if cfg!(debug_assertions) {
            self.text_positions
                .insert(self.stack_cur.pos + self.stack_pos - u32::from(pos));
        }
        let v = self.string_mut(pos);
        let s = String::new();
        unsafe {
            core::ptr::write(v, s);
        }
    }

    pub fn var_text(&mut self) {
        let pos = self.code::<u16>();
        let new_value = Str::new(self.get_var::<String>(pos));
        self.put_stack(new_value);
    }

    pub fn arg_text(&mut self) {
        let pos = self.code::<u16>();
        let new_value = *self.get_var::<Str>(pos);
        self.put_stack(new_value);
    }

    pub fn put_text(&mut self) {
        let pos = self.code::<u16>();
        let str_val = self.string(); // pops 16B Str; stack_pos -= 16
        self.put_var(pos, str_val); // writes Str at (stack_pos + 16 - pos) = elem_abs
    }

    pub fn format_int(&mut self) {
        let pos = self.code::<u16>();
        let radix = self.code::<u8>();
        let token = self.code::<u8>();
        let plus = self.code::<bool>();
        let note = self.code::<bool>();
        let dir = self.code::<i8>();
        let width = *self.get_stack::<i64>();
        let val = *self.get_stack::<i64>();
        // Plan-07 phase 4e.3 — consume the fault tag set by the
        // 4e.1 format-scope swap's preceding `OpTagFault`.  When the
        // value is the i64 null sentinel AND a tag is set, render
        // `null(<tag>)` instead of bare `null`.  Take + drop the
        // tag unconditionally so it can never leak to a downstream
        // interpolation in the same format string.
        let tag = self.database.take_format_fault();
        if val == i64::MIN
            && let Some(label_str) = tag
        {
            let label = format!("null({label_str})");
            let s = self.string_mut(pos - 16);
            ops::format_text(s, &label, width, 1, token);
            return;
        }
        let s = self.string_mut(pos - 16);
        ops::format_long(s, val, radix, width, token, plus, note, dir);
    }

    pub fn format_stack_int(&mut self) {
        let pos = self.code::<u16>();
        let radix = self.code::<u8>();
        let token = self.code::<u8>();
        let plus = self.code::<bool>();
        let note = self.code::<bool>();
        let dir = self.code::<i8>();
        let width = *self.get_stack::<i64>();
        let val = *self.get_stack::<i64>();
        let tag = self.database.take_format_fault();
        if val == i64::MIN
            && let Some(label_str) = tag
        {
            let label = format!("null({label_str})");
            let s = self.string_ref_mut(pos - 16);
            ops::format_text(s, &label, width, 1, token);
            return;
        }
        let s = self.string_ref_mut(pos - 16);
        ops::format_long(s, val, radix, width, token, plus, note, dir);
    }

    pub fn format_float(&mut self) {
        let pos = self.code::<u16>();
        let dir = self.code::<i8>();
        let precision = *self.get_stack::<i64>();
        let width = *self.get_stack::<i64>();
        let val = *self.get_stack::<f64>();
        let s = self.string_mut(pos - 24);
        ops::format_float(s, val, width, precision, dir);
    }

    pub fn format_stack_float(&mut self) {
        let pos = self.code::<u16>();
        let dir = self.code::<i8>();
        let precision = *self.get_stack::<i64>();
        let width = *self.get_stack::<i64>();
        let val = *self.get_stack::<f64>();
        let s = self.string_ref_mut(pos - 24); // f64(8)+i64(8)+i64(8) = 24 bytes popped
        ops::format_float(s, val, width, precision, dir);
    }

    pub fn format_single(&mut self) {
        let pos = self.code::<u16>();
        let dir = self.code::<i8>();
        let precision = *self.get_stack::<i64>();
        let width = *self.get_stack::<i64>();
        let val = *self.get_stack::<f32>();
        // @PLAN53 cluster 2 / S4: N = stepped span of the popped i64+i64+f32
        // (20 off; 24 aligned — the f32 rounds 4->8).
        let n = (self.stack_step(8) + self.stack_step(8) + self.stack_step(4)) as u16;
        let s = self.string_mut(pos - n);
        ops::format_single(s, val, width, precision, dir);
    }

    pub fn format_stack_single(&mut self) {
        let pos = self.code::<u16>();
        let dir = self.code::<i8>();
        let precision = *self.get_stack::<i64>();
        let width = *self.get_stack::<i64>();
        let val = *self.get_stack::<f32>();
        // @PLAN53 cluster 2 / S4: stepped span of popped i64+i64+f32 (20/24).
        let n = (self.stack_step(8) + self.stack_step(8) + self.stack_step(4)) as u16;
        let s = self.string_ref_mut(pos - n);
        ops::format_single(s, val, width, precision, dir);
    }

    pub fn format_text(&mut self) {
        let pos = self.code::<u16>();
        let dir = self.code::<i8>();
        let token = self.code::<u8>();
        let width = *self.get_stack::<i64>();
        let val = self.string();
        let s = self.string_mut(pos - 8 - size_ptr() as u16);
        ops::format_text(s, val.str(), width, dir, token);
    }

    pub fn format_stack_text(&mut self) {
        let pos = self.code::<u16>();
        let dir = self.code::<i8>();
        let token = self.code::<u8>();
        let width = *self.get_stack::<i64>();
        let val = self.string();
        let s = self.string_ref_mut(pos - 8 - size_ptr() as u16);
        ops::format_text(s, val.str(), width, dir, token);
    }
}
