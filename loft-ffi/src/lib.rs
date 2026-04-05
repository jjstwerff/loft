// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! # loft-ffi
//!
//! Helpers for writing loft native extension cdylibs.
//!
//! ## Quick start
//!
//! ```rust,ignore
//! use loft_ffi::{LoftStr, ret, text};
//!
//! #[unsafe(no_mangle)]
//! pub extern "C" fn n_greet(name_ptr: *const u8, name_len: usize) -> LoftStr {
//!     let name = unsafe { text(name_ptr, name_len) };
//!     ret(format!("Hello, {name}!"))
//! }
//! ```

use std::cell::RefCell;

// ── Null sentinels ─────────────────────────────────────────────────────

/// Null sentinel for `integer` (loft `i32`).
pub const NULL_INT: i32 = i32::MIN;

/// Null sentinel for `long` (loft `i64`).
pub const NULL_LONG: i64 = i64::MIN;

// ── LoftRef: opaque store reference ─────────────────────────────────────

/// Opaque reference to a loft store object (struct, vector, collection).
///
/// The cdylib receives this as an opaque handle.  It cannot dereference
/// the fields — only the interpreter can.  Pass it back to other loft
/// native functions unchanged, or check [`is_null`](LoftRef::is_null).
///
/// Layout matches `DbRef` in the interpreter (`u16 + u32 + u32`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LoftRef {
    pub store_nr: u16,
    pub rec: u32,
    pub pos: u32,
}

impl LoftRef {
    /// A null reference (no object).
    pub const NULL: Self = Self {
        store_nr: 0,
        rec: 0,
        pos: 0,
    };

    /// Returns `true` if this reference points to nothing.
    #[must_use]
    pub fn is_null(&self) -> bool {
        self.rec == 0 && self.pos == 0
    }
}

// SAFETY: LoftRef is a plain-old-data handle.  The pointed-to store
// data is only accessed by the interpreter on the calling thread.
unsafe impl Send for LoftRef {}
unsafe impl Sync for LoftRef {}

// ── LoftStore: direct field access to store memory ─────────────────────

/// Handle to a loft store's contiguous memory buffer.
///
/// Provides direct read/write access to struct fields via pointer
/// arithmetic.  The cdylib receives this as the first C-ABI argument
/// when any parameter is a `LoftRef`.
///
/// # Safety contract
///
/// - Reads and writes to **existing** records are safe for the duration
///   of the C-ABI call (the interpreter does not reallocate while the
///   cdylib is running).
/// - The pointer becomes invalid after the call returns — do not cache it.
/// - Field offsets are stable for the process lifetime (computed once at
///   parse time).
/// Opaque context pointer passed to callback functions.
/// The cdylib must not dereference or inspect this — just pass it through.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LoftStoreCtx {
    _opaque: *mut (),
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LoftStore {
    /// Base pointer to the store's memory buffer.
    pub ptr: *mut u8,
    /// Store capacity in 8-byte words (for bounds checking).
    pub size: u32,
    /// Opaque context for callbacks (interpreter-side `&mut Stores` + store_nr).
    pub ctx: LoftStoreCtx,
    /// Allocate `words` 8-byte words in the store. Returns the new record number.
    /// **After calling this, `ptr` may be stale — call `reload()` immediately.**
    pub claim_fn: Option<unsafe extern "C" fn(LoftStoreCtx, u32) -> u32>,
    /// Refresh `ptr` and `size` after an allocation that may have reallocated.
    pub reload_fn: Option<unsafe extern "C" fn(LoftStoreCtx, *mut *mut u8, *mut u32)>,
    /// Resize record `rec` to `words` 8-byte words. Returns the (possibly new) record number.
    /// **After calling this, `ptr` may be stale — call `reload()` immediately.**
    pub resize_fn: Option<unsafe extern "C" fn(LoftStoreCtx, u32, u32) -> u32>,
}

// SAFETY: The store is only accessed from the interpreter's thread
// during the C-ABI call.
unsafe impl Send for LoftStore {}
unsafe impl Sync for LoftStore {}

impl LoftStore {
    /// Read an `i32` field (loft `integer`).
    ///
    /// # Safety
    /// `rec`, `pos`, `offset` must point to a valid i32 within the store.
    #[inline]
    pub unsafe fn get_int(&self, rec: u32, pos: u32, offset: u16) -> i32 {
        unsafe {
            self.ptr
                .add(rec as usize * 8 + pos as usize + offset as usize)
                .cast::<i32>()
                .read_unaligned()
        }
    }

    /// Write an `i32` field.
    ///
    /// # Safety
    /// Same as `get_int`.
    #[inline]
    pub unsafe fn set_int(&self, rec: u32, pos: u32, offset: u16, val: i32) {
        unsafe {
            self.ptr
                .add(rec as usize * 8 + pos as usize + offset as usize)
                .cast::<i32>()
                .write_unaligned(val);
        }
    }

    /// Read an `i64` field (loft `long`).
    #[inline]
    pub unsafe fn get_long(&self, rec: u32, pos: u32, offset: u16) -> i64 {
        unsafe {
            self.ptr
                .add(rec as usize * 8 + pos as usize + offset as usize)
                .cast::<i64>()
                .read_unaligned()
        }
    }

    /// Write an `i64` field.
    #[inline]
    pub unsafe fn set_long(&self, rec: u32, pos: u32, offset: u16, val: i64) {
        unsafe {
            self.ptr
                .add(rec as usize * 8 + pos as usize + offset as usize)
                .cast::<i64>()
                .write_unaligned(val);
        }
    }

    /// Read an `f64` field (loft `float`).
    #[inline]
    pub unsafe fn get_float(&self, rec: u32, pos: u32, offset: u16) -> f64 {
        unsafe {
            self.ptr
                .add(rec as usize * 8 + pos as usize + offset as usize)
                .cast::<f64>()
                .read_unaligned()
        }
    }

    /// Write an `f64` field.
    #[inline]
    pub unsafe fn set_float(&self, rec: u32, pos: u32, offset: u16, val: f64) {
        unsafe {
            self.ptr
                .add(rec as usize * 8 + pos as usize + offset as usize)
                .cast::<f64>()
                .write_unaligned(val);
        }
    }

    /// Read a `u8` field (loft `boolean`, simple enum tag).
    #[inline]
    pub unsafe fn get_byte(&self, rec: u32, pos: u32, offset: u16) -> u8 {
        unsafe {
            *self
                .ptr
                .add(rec as usize * 8 + pos as usize + offset as usize)
        }
    }

    /// Write a `u8` field.
    #[inline]
    pub unsafe fn set_byte(&self, rec: u32, pos: u32, offset: u16, val: u8) {
        unsafe {
            *self
                .ptr
                .add(rec as usize * 8 + pos as usize + offset as usize) = val;
        }
    }

    /// Read a text field.  Returns `(ptr, len)` pointing into store memory.
    /// The pointer is valid until the C-ABI call returns.
    /// Returns `(null, 0)` for null text references.
    #[inline]
    pub unsafe fn get_text(&self, rec: u32, pos: u32, offset: u16) -> (*const u8, usize) {
        let str_rec = unsafe { self.get_int(rec, pos, offset) } as u32;
        if str_rec == 0 {
            return (std::ptr::null(), 0);
        }
        let len = unsafe { self.get_int(str_rec, 0, 4) } as usize;
        let ptr = unsafe { self.ptr.add(str_rec as usize * 8 + 8) };
        (ptr, len)
    }

    /// Read a sub-reference field (struct field that is itself a `LoftRef`).
    /// The returned ref's `store_nr` is copied from the parent.
    #[inline]
    pub unsafe fn get_ref(&self, store_nr: u16, rec: u32, pos: u32, offset: u16) -> LoftRef {
        let sub_rec = unsafe { self.get_int(rec, pos, offset) } as u32;
        LoftRef {
            store_nr,
            rec: sub_rec,
            pos: 8,
        }
    }

    // ── Allocation helpers ────────────────────────────────────────────

    /// The store number this handle operates on (encoded in the context).
    #[inline]
    #[must_use]
    pub fn store_nr(&self) -> u16 {
        self.ctx._opaque as usize as u16
    }

    /// Refresh `ptr` and `size` from the interpreter after a potential reallocation.
    ///
    /// Call this after every `claim()` or `resize()` — the raw pointer may
    /// have moved due to store growth.
    ///
    /// # Panics
    /// Panics if `reload_fn` is not set (store was created without callbacks).
    #[inline]
    pub unsafe fn reload(&mut self) {
        let f = self.reload_fn.expect("LoftStore: reload_fn not set");
        unsafe { f(self.ctx, &mut self.ptr, &mut self.size) };
    }

    /// Allocate `words` 8-byte words in the store. Returns the new record number.
    ///
    /// **Automatically reloads `ptr`/`size`** — safe to read/write immediately after.
    ///
    /// # Panics
    /// Panics if `claim_fn` is not set.
    pub unsafe fn claim(&mut self, words: u32) -> u32 {
        let f = self.claim_fn.expect("LoftStore: claim_fn not set");
        let rec = unsafe { f(self.ctx, words) };
        unsafe { self.reload() };
        rec
    }

    /// Resize record `rec` to `words` 8-byte words. Returns the (possibly new)
    /// record number — the record may have been relocated.
    ///
    /// **Automatically reloads `ptr`/`size`** — safe to read/write immediately after.
    ///
    /// # Panics
    /// Panics if `resize_fn` is not set.
    pub unsafe fn resize(&mut self, rec: u32, words: u32) -> u32 {
        let f = self.resize_fn.expect("LoftStore: resize_fn not set");
        let new_rec = unsafe { f(self.ctx, rec, words) };
        unsafe { self.reload() };
        new_rec
    }

    /// Allocate an empty struct record of `words` 8-byte words.
    ///
    /// Returns a `LoftRef` pointing to the start of the data area (pos = 8,
    /// skipping the record header).
    pub unsafe fn alloc_record(&mut self, words: u32) -> LoftRef {
        let rec = unsafe { self.claim(words) };
        LoftRef {
            store_nr: self.store_nr(),
            rec,
            pos: 8,
        }
    }

    /// Allocate an empty vector with space for `capacity` elements.
    ///
    /// `elem_size` is the element size in bytes (4 for integer/single,
    /// 8 for long/float, or the struct record-ref size of 4).
    ///
    /// The vector starts with length 0. Use `vector_push_*` to append.
    /// The minimum allocation is 11 elements (matching interpreter convention).
    pub unsafe fn alloc_vector(&mut self, elem_size: u32, capacity: u32) -> LoftRef {
        let alloc_count = capacity.max(11);
        let words = (alloc_count * elem_size + 15) / 8;
        let vec_rec = unsafe { self.claim(words) };
        // Initialize length = 0 (at byte offset 4 within the record).
        unsafe { self.set_int(vec_rec, 0, 4, 0) };
        LoftRef {
            store_nr: self.store_nr(),
            rec: vec_rec,
            pos: 8,
        }
    }

    /// Current number of elements in a vector.
    ///
    /// # Safety
    /// `vec` must point to a valid vector record in this store.
    #[inline]
    pub unsafe fn vector_len(&self, vec: &LoftRef) -> u32 {
        unsafe { self.get_int(vec.rec, 0, 4) as u32 }
    }

    /// Ensure the vector has room for one more element, resizing if needed.
    /// Returns the (possibly updated) `vec_rec` — the record may have moved.
    unsafe fn vector_grow(&mut self, vec_rec: u32, elem_size: u32) -> u32 {
        let length = unsafe { self.get_int(vec_rec, 0, 4) as u32 };
        let needed_words = ((length + 1) * elem_size + 15) / 8;
        let new_rec = unsafe { self.resize(vec_rec, needed_words) };
        // resize() already called reload(), ptr is fresh
        new_rec
    }

    /// Append an `i32` to a vector of integers.
    ///
    /// `vec` is updated in place if the record moves during resize.
    ///
    /// # Safety
    /// `vec` must point to a valid `vector<integer>` in this store.
    pub unsafe fn vector_push_int(&mut self, vec: &mut LoftRef, val: i32) {
        let new_rec = unsafe { self.vector_grow(vec.rec, 4) };
        vec.rec = new_rec;
        let length = unsafe { self.get_int(new_rec, 0, 4) as u32 };
        // Write value at data offset: 8 + length * 4
        unsafe { self.set_int(new_rec, 8 + length * 4, 0, val) };
        // Increment length
        unsafe { self.set_int(new_rec, 0, 4, length as i32 + 1) };
    }

    /// Append an `i64` to a vector of longs.
    ///
    /// `vec` is updated in place if the record moves during resize.
    ///
    /// # Safety
    /// `vec` must point to a valid `vector<long>` in this store.
    pub unsafe fn vector_push_long(&mut self, vec: &mut LoftRef, val: i64) {
        let new_rec = unsafe { self.vector_grow(vec.rec, 8) };
        vec.rec = new_rec;
        let length = unsafe { self.get_int(new_rec, 0, 4) as u32 };
        unsafe { self.set_long(new_rec, 8 + length * 8, 0, val) };
        unsafe { self.set_int(new_rec, 0, 4, length as i32 + 1) };
    }

    /// Append an `f64` to a vector of floats.
    ///
    /// `vec` is updated in place if the record moves during resize.
    ///
    /// # Safety
    /// `vec` must point to a valid `vector<float>` in this store.
    pub unsafe fn vector_push_float(&mut self, vec: &mut LoftRef, val: f64) {
        let new_rec = unsafe { self.vector_grow(vec.rec, 8) };
        vec.rec = new_rec;
        let length = unsafe { self.get_int(new_rec, 0, 4) as u32 };
        unsafe { self.set_float(new_rec, 8 + length * 8, 0, val) };
        unsafe { self.set_int(new_rec, 0, 4, length as i32 + 1) };
    }

    /// Append one element of `elem_size` bytes to a vector.
    ///
    /// Returns the byte position of the new element within the vector record.
    /// The caller writes the element's fields at `(vec.rec, returned_pos, field_offset)`.
    ///
    /// # Safety
    /// `vec` must point to a valid vector in this store.
    pub unsafe fn vector_push(&mut self, vec: &mut LoftRef, elem_size: u32) -> u32 {
        let new_rec = unsafe { self.vector_grow(vec.rec, elem_size) };
        vec.rec = new_rec;
        let length = unsafe { self.get_int(new_rec, 0, 4) as u32 };
        let elem_pos = 8 + length * elem_size;
        unsafe { self.set_int(new_rec, 0, 4, length as i32 + 1) };
        elem_pos
    }

    // ── Bulk data helpers ─────────────────────────────────────────────

    /// Copy raw bytes into store memory at a specific position.
    ///
    /// # Safety
    /// The destination `rec * 8 + pos` must be within the store's allocated area.
    /// `src` must point to `len` readable bytes.
    #[inline]
    pub unsafe fn write_bytes(&self, rec: u32, pos: u32, src: *const u8, len: usize) {
        let dst = unsafe { self.ptr.add(rec as usize * 8 + pos as usize) };
        unsafe { std::ptr::copy_nonoverlapping(src, dst, len) };
    }

    /// Allocate a vector and fill it with raw byte data.
    ///
    /// Creates a vector of `count` elements (each `elem_size` bytes) and copies
    /// `data` directly into the vector's data area. The data length must be
    /// exactly `count * elem_size`.
    ///
    /// Returns a `LoftRef` to the vector record.
    ///
    /// # Safety
    /// `data` must point to `count * elem_size` readable bytes.
    pub unsafe fn alloc_vector_from_bytes(
        &mut self,
        elem_size: u32,
        count: u32,
        data: *const u8,
        data_len: usize,
    ) -> LoftRef {
        let mut vec = unsafe { self.alloc_vector(elem_size, count) };
        // Set length to count (alloc_vector starts at 0).
        unsafe { self.set_int(vec.rec, 0, 4, count as i32) };
        // Bulk copy data after the 8-byte vector header.
        if data_len > 0 {
            unsafe { self.write_bytes(vec.rec, 8, data, data_len) };
        }
        vec.pos = 8;
        vec
    }

    /// Allocate a text string in the store and set a struct field to point to it.
    ///
    /// The text record layout is: `[header(4)] [length(4)] [utf8-bytes...]`.
    /// The field at `(rec, pos, offset)` is set to the text record number.
    ///
    /// # Safety
    /// `rec`, `pos`, `offset` must point to a valid i32 field in the store.
    pub unsafe fn set_text(&mut self, rec: u32, pos: u32, offset: u16, val: &str) {
        let words = ((val.len() + 15) / 8) as u32;
        let str_rec = unsafe { self.claim(words) };
        // Write string length at str_rec + 4 bytes.
        unsafe { self.set_int(str_rec, 0, 4, val.len() as i32) };
        // Copy UTF-8 bytes starting at str_rec * 8 + 8.
        if !val.is_empty() {
            unsafe { self.write_bytes(str_rec, 8, val.as_ptr(), val.len()) };
        }
        // Set the field to point to the text record.
        unsafe { self.set_int(rec, pos, offset, str_rec as i32) };
    }
}

// ── LoftStr: safe text return ──────────────────────────────────────────

/// A `#[repr(C)]` text value returned from native functions.
///
/// The pointer is borrowed — valid until the next call to [`ret`] on the
/// same thread.  The interpreter copies immediately after the call.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LoftStr {
    pub ptr: *const u8,
    pub len: usize,
}

impl LoftStr {
    /// Empty text (null pointer, zero length).
    pub const EMPTY: Self = Self {
        ptr: std::ptr::null(),
        len: 0,
    };
}

// SAFETY: LoftStr is only used as a return value within a single
// function call scope.  The pointer is never sent across threads.
unsafe impl Send for LoftStr {}
unsafe impl Sync for LoftStr {}

thread_local! {
    static RET_BUF: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Store `s` in a thread-local buffer and return a borrowed view.
///
/// The returned [`LoftStr`] is valid until the next call to `ret` on
/// the same thread.  The interpreter copies the bytes into its own
/// storage immediately after the C-ABI call returns.
///
/// # Example
/// ```rust,ignore
/// use loft_ffi::{ret, LoftStr};
///
/// #[unsafe(no_mangle)]
/// pub extern "C" fn n_hello() -> LoftStr {
///     ret("Hello!".to_string())
/// }
/// ```
#[must_use]
pub fn ret(s: String) -> LoftStr {
    RET_BUF.with(|buf| {
        *buf.borrow_mut() = s;
        let b = buf.borrow();
        LoftStr {
            ptr: b.as_ptr(),
            len: b.len(),
        }
    })
}

/// Return a borrowed view of an existing `&str`.
///
/// Use this when the data already lives in a thread-local or static
/// and doesn't need to be copied into the return buffer.
///
/// # Safety
/// The `&str` must remain valid until the interpreter has copied the
/// bytes (i.e. until the `extern "C"` function returns).
#[must_use]
pub fn ret_ref(s: &str) -> LoftStr {
    LoftStr {
        ptr: s.as_ptr(),
        len: s.len(),
    }
}

// ── Text parameter helpers ─────────────────────────────────────────────

/// Convert a `(*const u8, usize)` C-ABI text parameter to `&str`.
///
/// # Safety
/// The caller must ensure `ptr` points to valid UTF-8 of length `len`.
/// This is guaranteed by the loft interpreter for all `text` arguments.
#[must_use]
pub unsafe fn text<'a>(ptr: *const u8, len: usize) -> &'a str {
    unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len)) }
}

/// Convert a `(*const u8, usize)` pair to `Option<&str>`.
///
/// Returns `None` when `ptr` is null or `len` is zero.
///
/// # Safety
/// Same as [`text`].
#[must_use]
pub unsafe fn text_opt<'a>(ptr: *const u8, len: usize) -> Option<&'a str> {
    if ptr.is_null() || len == 0 {
        None
    } else {
        Some(unsafe { text(ptr, len) })
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a read/write test store with no callbacks (allocation not supported).
    fn test_store(buf: &mut [u8]) -> LoftStore {
        LoftStore {
            ptr: buf.as_mut_ptr(),
            size: (buf.len() / 8) as u32,
            ctx: LoftStoreCtx { _opaque: std::ptr::null_mut() },
            claim_fn: None,
            reload_fn: None,
            resize_fn: None,
        }
    }

    #[test]
    fn ret_returns_borrowed_view() {
        let s = ret("hello".to_string());
        assert_eq!(s.len, 5);
        assert!(!s.ptr.is_null());
        let slice = unsafe { std::slice::from_raw_parts(s.ptr, s.len) };
        assert_eq!(slice, b"hello");
    }

    #[test]
    fn ret_ref_borrows_existing() {
        let data = "world";
        let s = ret_ref(data);
        assert_eq!(s.len, 5);
        assert_eq!(s.ptr, data.as_ptr());
    }

    #[test]
    fn empty_is_null() {
        assert!(LoftStr::EMPTY.ptr.is_null());
        assert_eq!(LoftStr::EMPTY.len, 0);
    }

    #[test]
    fn text_converts() {
        let data = "test";
        let s = unsafe { text(data.as_ptr(), data.len()) };
        assert_eq!(s, "test");
    }

    #[test]
    fn text_opt_none_on_null() {
        assert!(unsafe { text_opt(std::ptr::null(), 0) }.is_none());
    }

    #[test]
    fn text_opt_some_on_valid() {
        let data = "ok";
        assert_eq!(
            unsafe { text_opt(data.as_ptr(), data.len()) },
            Some("ok")
        );
    }

    #[test]
    fn loft_ref_null() {
        assert!(LoftRef::NULL.is_null());
    }

    #[test]
    fn loft_ref_non_null() {
        let r = LoftRef { store_nr: 1, rec: 42, pos: 8 };
        assert!(!r.is_null());
    }

    #[test]
    fn loft_ref_size() {
        // Must be 10 bytes data + padding; repr(C) gives predictable layout.
        assert!(std::mem::size_of::<LoftRef>() <= 12);
    }

    #[test]
    fn loft_store_get_set_int() {
        // Simulate a store: 16 words = 128 bytes.
        let mut buf = vec![0u8; 128];
        let store = test_store(&mut buf);
        // Write i32 at rec=2, pos=8, offset=4  → byte 2*8+8+4 = 28
        unsafe { store.set_int(2, 8, 4, 42) };
        assert_eq!(unsafe { store.get_int(2, 8, 4) }, 42);
    }

    #[test]
    fn loft_store_get_set_long() {
        let mut buf = vec![0u8; 128];
        let store = test_store(&mut buf);
        unsafe { store.set_long(2, 8, 0, 123_456_789_012) };
        assert_eq!(unsafe { store.get_long(2, 8, 0) }, 123_456_789_012);
    }

    #[test]
    fn loft_store_get_set_float() {
        let mut buf = vec![0u8; 128];
        let store = test_store(&mut buf);
        unsafe { store.set_float(2, 8, 0, 3.14) };
        let v = unsafe { store.get_float(2, 8, 0) };
        assert!((v - 3.14).abs() < 1e-10);
    }

    #[test]
    fn loft_store_get_set_byte() {
        let mut buf = vec![0u8; 128];
        let store = test_store(&mut buf);
        unsafe { store.set_byte(2, 8, 0, 255) };
        assert_eq!(unsafe { store.get_byte(2, 8, 0) }, 255);
    }
}
