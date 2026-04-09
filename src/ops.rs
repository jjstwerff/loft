// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Pure Rust scalar operations used by the bytecode executor (`fill.rs`) and
//! by native function implementations (`native.rs`).
//!
//! ## Naming conventions
//!
//! * `op_cast_X_from_Y`  — narrowing or lossy conversion (may truncate or clamp).
//!   Example: `op_cast_int_from_long` truncates a 64-bit value to 32 bits.
//! * `op_conv_X_from_Y`  — widening or safe conversion (no precision loss).
//!   Example: `op_conv_long_from_int` zero-extends a 32-bit integer to 64 bits.
//! * `op_negate_X`       — unary negation (single operand; not a minimum-of-two).
//! * `op_abs_X`          — absolute value.
//! * `op_<verb>_X`       — binary arithmetic (`add`, `min`, `mul`, `div`, `rem`, …).
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_precision_loss)]
#![allow(dead_code)]
#[cfg(feature = "random")]
use rand_core::{RngCore, SeedableRng};
#[cfg(feature = "random")]
use rand_pcg::Pcg64;
#[cfg(feature = "random")]
use std::cell::RefCell;
use std::cmp::Ordering;

#[cfg(feature = "random")]
thread_local! {
    static RNG: RefCell<Pcg64> = RefCell::new(Pcg64::seed_from_u64(12345));
}

/// In debug builds, use checked arithmetic and panic on overflow.
/// In release builds, use unchecked arithmetic for speed.
macro_rules! checked_int {
    ($checked:expr, $op:expr, $v1:expr, $v2:expr) => {{
        #[cfg(debug_assertions)]
        {
            let r = $checked.unwrap_or_else(|| panic!("integer overflow: {} {} {}", $v1, $op, $v2));
            assert!(
                r != i32::MIN,
                "integer null-sentinel collision: {} {} {} = i32::MIN",
                $v1,
                $op,
                $v2
            );
            r
        }
        #[cfg(not(debug_assertions))]
        {
            $checked.unwrap_or(i32::MIN)
        }
    }};
}

macro_rules! checked_long {
    ($checked:expr, $op:expr, $v1:expr, $v2:expr) => {{
        #[cfg(debug_assertions)]
        {
            let r = $checked.unwrap_or_else(|| panic!("long overflow: {} {} {}", $v1, $op, $v2));
            assert!(
                r != i64::MIN,
                "long null-sentinel collision: {} {} {} = i64::MIN",
                $v1,
                $op,
                $v2
            );
            r
        }
        #[cfg(not(debug_assertions))]
        {
            $checked.unwrap_or(i64::MIN)
        }
    }};
}

macro_rules! sentinel_int {
    ($expr:expr, $op:expr, $v1:expr, $v2:expr) => {{
        let r = $expr;
        #[cfg(debug_assertions)]
        assert!(
            r != i32::MIN,
            "integer null-sentinel collision: {} {} {} = i32::MIN",
            $v1,
            $op,
            $v2
        );
        r
    }};
}

macro_rules! sentinel_long {
    ($expr:expr, $op:expr, $v1:expr, $v2:expr) => {{
        let r = $expr;
        #[cfg(debug_assertions)]
        assert!(
            r != i64::MIN,
            "long null-sentinel collision: {} {} {} = i64::MIN",
            $v1,
            $op,
            $v2
        );
        r
    }};
}

/// Return a random integer in `[lo, hi]` (inclusive).
/// Returns `i32::MIN` (null) if `lo > hi` or if either bound is null.
#[cfg(feature = "random")]
#[must_use]
pub fn rand_int(lo: i32, hi: i32) -> i32 {
    if lo == i32::MIN || hi == i32::MIN || lo > hi {
        return i32::MIN;
    }
    let range = (i64::from(hi) - i64::from(lo) + 1) as u64;
    let r = RNG.with(|rng| rng.borrow_mut().next_u64());
    lo + (r % range) as i32
}

/// WASM fallback: delegate to the JS host RNG when `random` crate is not available.
#[cfg(all(feature = "wasm", not(feature = "random")))]
#[must_use]
pub fn rand_int(lo: i32, hi: i32) -> i32 {
    if lo == i32::MIN || hi == i32::MIN || lo > hi {
        return i32::MIN;
    }
    crate::wasm::host_random_int(lo, hi)
}

/// Reseed the thread-local RNG.
#[cfg(feature = "random")]
pub fn rand_seed(seed: i64) {
    RNG.with(|rng| *rng.borrow_mut() = Pcg64::seed_from_u64(seed as u64));
}

/// WASM fallback: delegate seed to the JS host RNG.
#[cfg(all(feature = "wasm", not(feature = "random")))]
pub fn rand_seed(seed: i64) {
    crate::wasm::host_random_seed(seed);
}

/// Fisher-Yates shuffle of a mutable slice of `i32`.
#[cfg(feature = "random")]
pub fn shuffle_ints(v: &mut [i32]) {
    let n = v.len();
    for i in (1..n).rev() {
        let j = RNG.with(|rng| rng.borrow_mut().next_u64()) as usize % (i + 1);
        v.swap(i, j);
    }
}

/// Fisher-Yates shuffle via the WASM host-bridge RNG.
/// Used by `n_rand_indices` when `feature = "random"` is not available.
#[cfg(all(feature = "wasm", not(feature = "random")))]
pub fn shuffle_ints(v: &mut [i32]) {
    let n = v.len();
    for i in (1..n).rev() {
        let j = crate::wasm::host_random_int(0, i as i32) as usize;
        v.swap(i, j);
    }
}

#[must_use]
pub fn text_character(val: &str, from: i32) -> char {
    let len = val.len() as i32;
    let mut idx = if from < 0 { from + len } else { from };
    if idx < 0 || idx >= len {
        return char::from(0);
    }
    let mut b = val.as_bytes()[idx as usize];
    while b & 0xC0 == 0x80 && idx > 0 {
        idx -= 1;
        b = val.as_bytes()[idx as usize];
    }
    val[idx as usize..].chars().next().unwrap_or(char::from(0))
}

#[must_use]
pub fn sub_text(val: &str, from: i32, till: i32) -> &str {
    let size = val.len() as i32;
    let mut f = if from < 0 { from + size } else { from };
    let mut t = if till == i32::MIN {
        f + 1
    } else if till < 0 {
        till + size
    } else if till > size {
        size
    } else {
        till
    };
    if f < 0 || f > size || t < f || t > size {
        return "";
    }
    // when till is inside a UTF-8 token: increase it
    while t < size && !val.is_char_boundary(t as usize) {
        t += 1;
    }
    // when from is inside a UTF-8 token: decrease it
    while f > 0 && !val.is_char_boundary(f as usize) {
        f -= 1;
    }
    &val[f as usize..t as usize]
}

#[inline]
#[must_use]
pub fn to_char(val: i32) -> char {
    unsafe { char::from_u32_unchecked(val as u32) }
}

#[inline]
pub fn format_text(s: &mut String, val: &str, width: i32, dir: i8, token: u8) {
    // dir=2 means "unset default"; text defaults to left-align (-1)
    let dir = if dir == 2 { -1 } else { dir };
    let mut tokens = width as usize;
    for _ in val.chars() {
        if tokens == 0 {
            break;
        }
        tokens -= 1;
    }
    match dir.cmp(&0) {
        Ordering::Less => {
            *s += val;
            while tokens > 0 {
                s.push(token as char);
                tokens -= 1;
            }
        }
        Ordering::Greater => {
            while tokens > 0 {
                s.push(token as char);
                tokens -= 1;
            }
            *s += val;
        }
        Ordering::Equal => {
            let mut ct = 0;
            while ct < tokens / 2 {
                s.push(token as char);
                ct += 1;
            }
            *s += val;
            while ct < tokens {
                s.push(token as char);
                ct += 1;
            }
        }
    }
}
#[inline]
#[must_use]
pub fn op_abs_long(val: i64) -> i64 {
    if val == i64::MIN { val } else { val.abs() }
}

#[inline]
#[must_use]
pub fn op_negate_long(val: i64) -> i64 {
    if val == i64::MIN { val } else { -val }
}

#[inline]
#[must_use]
pub fn op_cast_int_from_long(val: i64) -> i32 {
    if val == i64::MIN {
        i32::MIN
    } else {
        val as i32
    }
}

#[inline]
#[must_use]
pub fn op_cast_int_from_single(val: f32) -> i32 {
    if val.is_nan() { i32::MIN } else { val as i32 }
}

#[inline]
#[must_use]
pub fn op_cast_long_from_single(val: f32) -> i64 {
    if val.is_nan() { i64::MIN } else { val as i64 }
}

#[inline]
#[must_use]
pub fn op_cast_int_from_float(val: f64) -> i32 {
    if val.is_nan() { i32::MIN } else { val as i32 }
}

#[inline]
#[must_use]
pub fn op_cast_long_from_float(val: f64) -> i64 {
    if val.is_nan() { i64::MIN } else { val as i64 }
}

#[inline]
#[must_use]
pub fn op_conv_float_from_long(val: i64) -> f64 {
    if val == i64::MIN {
        f64::NAN
    } else {
        val as f64
    }
}

#[inline]
#[must_use]
pub fn op_conv_bool_from_long(val: i64) -> bool {
    val != i64::MIN
}

#[inline]
#[must_use]
pub fn op_add_long(v1: i64, v2: i64) -> i64 {
    if v1 != i64::MIN && v2 != i64::MIN {
        checked_long!(v1.checked_add(v2), "+", v1, v2)
    } else {
        i64::MIN
    }
}

#[inline]
#[must_use]
pub fn op_min_long(v1: i64, v2: i64) -> i64 {
    if v1 != i64::MIN && v2 != i64::MIN {
        checked_long!(v1.checked_sub(v2), "-", v1, v2)
    } else {
        i64::MIN
    }
}

#[inline]
#[must_use]
pub fn op_mul_long(v1: i64, v2: i64) -> i64 {
    if v1 != i64::MIN && v2 != i64::MIN {
        checked_long!(v1.checked_mul(v2), "*", v1, v2)
    } else {
        i64::MIN
    }
}

#[inline]
#[must_use]
pub fn op_div_long(v1: i64, v2: i64) -> i64 {
    if v1 != i64::MIN && v2 != i64::MIN && v2 != 0 {
        checked_long!(v1.checked_div(v2), "/", v1, v2)
    } else {
        i64::MIN
    }
}

#[inline]
#[must_use]
pub fn op_rem_long(v1: i64, v2: i64) -> i64 {
    if v1 != i64::MIN && v2 != i64::MIN && v2 != 0 {
        checked_long!(v1.checked_rem(v2), "%", v1, v2)
    } else {
        i64::MIN
    }
}

// ── O6: Non-null long variants ────────────────────────────────────────────
// Skip the i64::MIN sentinel check when both operands are known non-null
// (local variables with definite assignment).  Used by native codegen.

#[inline]
#[must_use]
pub fn op_add_long_nn(v1: i64, v2: i64) -> i64 {
    checked_long!(v1.checked_add(v2), "+", v1, v2)
}

#[inline]
#[must_use]
pub fn op_min_long_nn(v1: i64, v2: i64) -> i64 {
    checked_long!(v1.checked_sub(v2), "-", v1, v2)
}

#[inline]
#[must_use]
pub fn op_mul_long_nn(v1: i64, v2: i64) -> i64 {
    checked_long!(v1.checked_mul(v2), "*", v1, v2)
}

#[inline]
#[must_use]
pub fn op_div_long_nn(v1: i64, v2: i64) -> i64 {
    if v2 != 0 {
        checked_long!(v1.checked_div(v2), "/", v1, v2)
    } else {
        i64::MIN
    }
}

#[inline]
#[must_use]
pub fn op_rem_long_nn(v1: i64, v2: i64) -> i64 {
    if v2 != 0 {
        checked_long!(v1.checked_rem(v2), "%", v1, v2)
    } else {
        i64::MIN
    }
}

#[inline]
#[must_use]
pub fn op_neg_long_nn(v1: i64) -> i64 {
    checked_long!(v1.checked_neg(), "-", v1, 0)
}

#[inline]
#[must_use]
pub fn op_logical_and_long(v1: i64, v2: i64) -> i64 {
    if v1 != i64::MIN && v2 != i64::MIN {
        sentinel_long!(v1 & v2, "&", v1, v2)
    } else {
        i64::MIN
    }
}

#[inline]
#[must_use]
pub fn op_logical_or_long(v1: i64, v2: i64) -> i64 {
    if v1 != i64::MIN && v2 != i64::MIN {
        sentinel_long!(v1 | v2, "|", v1, v2)
    } else {
        i64::MIN
    }
}

#[inline]
#[must_use]
pub fn op_exclusive_or_long(v1: i64, v2: i64) -> i64 {
    if v1 != i64::MIN && v2 != i64::MIN {
        sentinel_long!(v1 ^ v2, "^", v1, v2)
    } else {
        i64::MIN
    }
}

#[inline]
#[must_use]
/// # Panics
/// In debug builds, panics if `v2` is outside `0..64`.
pub fn op_shift_left_long(v1: i64, v2: i64) -> i64 {
    if v1 != i64::MIN && v2 != i64::MIN {
        #[cfg(debug_assertions)]
        assert!(
            (0..64).contains(&v2),
            "long shift out of range: {v1} << {v2}"
        );
        sentinel_long!(v1 << v2, "<<", v1, v2)
    } else {
        i64::MIN
    }
}

#[inline]
#[must_use]
pub fn op_shift_right_long(v1: i64, v2: i64) -> i64 {
    if v1 != i64::MIN && v2 != i64::MIN {
        v1 >> v2
    } else {
        i64::MIN
    }
}

#[inline]
#[must_use]
pub fn op_abs_int(val: i32) -> i32 {
    if val == i32::MIN { val } else { val.abs() }
}

#[inline]
#[must_use]
pub fn op_negate_int(val: i32) -> i32 {
    if val == i32::MIN { val } else { -val }
}

#[inline]
#[must_use]
pub fn op_conv_long_from_int(val: i32) -> i64 {
    if val == i32::MIN {
        i64::MIN
    } else {
        i64::from(val)
    }
}

#[inline]
#[must_use]
pub fn op_conv_float_from_int(val: i32) -> f64 {
    if val == i32::MIN {
        f64::NAN
    } else {
        f64::from(val)
    }
}

#[inline]
#[must_use]
pub fn op_conv_single_from_int(val: i32) -> f32 {
    if val == i32::MIN {
        f32::NAN
    } else {
        val as f32
    }
}

#[inline]
#[must_use]
pub fn op_conv_bool_from_int(v: i32) -> bool {
    v != i32::MIN
}

#[inline]
#[must_use]
pub fn op_conv_bool_from_character(v: char) -> bool {
    // P132: callers must read raw bytes via `char::from_u32(...).unwrap_or('\0')`
    // (handled in `create.rs::generate_code_to`), so an invalid bit pattern
    // — including the `i32::MIN` (0x80000000) coroutine-exhaustion sentinel
    // pushed by `push_null_value` for `iterator<character>` — is mapped to
    // `'\0'` *before* this function is called. The check below therefore only
    // needs to recognise the explicit null character. Reading raw stack bytes
    // directly as `char` would be undefined behaviour: Rust assumes every
    // `char` is a valid Unicode scalar value, and the release-mode optimiser
    // would constant-fold the sentinel check away.
    v != '\0'
}

#[inline]
#[must_use]
pub fn op_add_int(v1: i32, v2: i32) -> i32 {
    if v1 != i32::MIN && v2 != i32::MIN {
        checked_int!(v1.checked_add(v2), "+", v1, v2)
    } else {
        i32::MIN
    }
}

#[inline]
#[must_use]
pub fn op_min_int(v1: i32, v2: i32) -> i32 {
    if v1 != i32::MIN && v2 != i32::MIN {
        checked_int!(v1.checked_sub(v2), "-", v1, v2)
    } else {
        i32::MIN
    }
}

#[inline]
#[must_use]
pub fn op_mul_int(v1: i32, v2: i32) -> i32 {
    if v1 != i32::MIN && v2 != i32::MIN {
        checked_int!(v1.checked_mul(v2), "*", v1, v2)
    } else {
        i32::MIN
    }
}

#[inline]
#[must_use]
pub fn op_div_int(v1: i32, v2: i32) -> i32 {
    if v1 != i32::MIN && v2 != i32::MIN && v2 != 0 {
        checked_int!(v1.checked_div(v2), "/", v1, v2)
    } else {
        i32::MIN
    }
}

#[inline]
#[must_use]
pub fn op_rem_int(v1: i32, v2: i32) -> i32 {
    if v1 != i32::MIN && v2 != i32::MIN && v2 != 0 {
        checked_int!(v1.checked_rem(v2), "%", v1, v2)
    } else {
        i32::MIN
    }
}

#[inline]
#[must_use]
pub fn op_logical_and_int(v1: i32, v2: i32) -> i32 {
    if v1 != i32::MIN && v2 != i32::MIN {
        sentinel_int!(v1 & v2, "&", v1, v2)
    } else {
        i32::MIN
    }
}

#[inline]
#[must_use]
pub fn op_logical_or_int(v1: i32, v2: i32) -> i32 {
    if v1 != i32::MIN && v2 != i32::MIN {
        sentinel_int!(v1 | v2, "|", v1, v2)
    } else {
        i32::MIN
    }
}

#[inline]
#[must_use]
pub fn op_exclusive_or_int(v1: i32, v2: i32) -> i32 {
    if v1 != i32::MIN && v2 != i32::MIN {
        sentinel_int!(v1 ^ v2, "^", v1, v2)
    } else {
        i32::MIN
    }
}

#[inline]
#[must_use]
/// # Panics
/// In debug builds, panics if `v2` is outside `0..32`.
pub fn op_shift_left_int(v1: i32, v2: i32) -> i32 {
    if v1 != i32::MIN && v2 != i32::MIN {
        #[cfg(debug_assertions)]
        assert!(
            (0..32).contains(&v2),
            "integer shift out of range: {v1} << {v2}"
        );
        sentinel_int!(v1 << v2, "<<", v1, v2)
    } else {
        i32::MIN
    }
}

#[inline]
#[must_use]
pub fn op_shift_right_int(v1: i32, v2: i32) -> i32 {
    if v1 != i32::MIN && v2 != i32::MIN {
        v1 >> v2
    } else {
        i32::MIN
    }
}

/**
Format an integer.
# Panics
When unknown radix values are asked.
*/
#[inline]
pub fn format_int(
    s: &mut String,
    val: i32,
    radix: u8,
    width: i32,
    token: u8,
    plus: bool,
    note: bool,
) {
    if val == i32::MIN {
        format_text(s, "null", width, 1, token);
        return;
    }
    let mut res = String::new();
    match radix {
        2 => {
            res += if note { "0b" } else { "" };
            write!(res, "{val:b}").unwrap();
        }
        8 => {
            res += if note { "0o" } else { "" };
            write!(res, "{val:o}").unwrap();
        }
        10 => {
            let sign = if val >= 0 {
                if plus { "+" } else { "" }
            } else {
                "-"
            };
            if token == b'0' && !sign.is_empty() {
                // Sign before zeros: "-01" not "0-1"
                *s += sign;
                write!(res, "{}", val.abs()).unwrap();
                format_text(s, &res, width - 1, 1, token);
                return;
            }
            res += sign;
            write!(res, "{}", val.abs()).unwrap();
        }
        16 => {
            res += if note { "0x" } else { "" };
            write!(res, "{val:x}").unwrap();
        }
        _ => panic!("Unknown radix"),
    }
    format_text(s, &res, width, 1, token);
}

/**
Format a long integer.
# Panics
When unknown radix values are asked.
*/
#[allow(clippy::too_many_arguments)]
#[inline]
pub fn format_long(
    s: &mut String,
    val: i64,
    radix: u8,
    width: i32,
    token: u8,
    plus: bool,
    note: bool,
    dir: i8,
) {
    // Numbers default to right-align; dir=-1 means "unset" from the parser
    // (left-align is the text default, but for numbers right-align is conventional).
    // Explicit `<` sets dir=-1, `^` sets dir=0, `>` sets dir=1.
    // We use dir=2 as "unset/default" from the parser, mapped to right-align here.
    let dir = if dir == 2 { 1 } else { dir };
    if val == i64::MIN {
        format_text(s, "null", width, dir, token);
        return;
    }
    let mut res = String::new();
    match radix {
        2 => {
            if note {
                res += "0b";
            }
            write!(res, "{val:b}").unwrap();
        }
        8 => {
            if note {
                res += "0o";
            }
            write!(res, "{val:o}").unwrap();
        }
        10 => {
            let sign = if val >= 0 {
                if plus { "+" } else { "" }
            } else {
                "-"
            };
            if token == b'0' && !sign.is_empty() {
                // Sign before zeros: "-01" not "0-1"
                *s += sign;
                write!(res, "{}", val.abs()).unwrap();
                format_text(s, &res, width - 1, dir, token);
                return;
            }
            res += sign;
            write!(res, "{}", val.abs()).unwrap();
        }
        16 => {
            res += if note { "0x" } else { "" };
            write!(res, "{val:x}").unwrap();
        }
        _ => panic!("Unknown radix"),
    }
    format_text(s, &res, width, dir, token);
}

use std::fmt::Write as _;

pub fn format_float(s: &mut String, val: f64, width: i32, precision: i32, dir: i8) {
    let dir = if dir == 2 { 1 } else { dir };
    let mut res = String::new();
    if precision >= 0 {
        write!(res, "{val:.p$}", p = precision as usize).unwrap();
    } else {
        write!(res, "{val}").unwrap();
    }
    format_text(s, &res, width, dir, b' ');
}

pub fn format_single(s: &mut String, val: f32, width: i32, precision: i32, dir: i8) {
    let dir = if dir == 2 { 1 } else { dir };
    let mut res = String::new();
    if precision >= 0 {
        write!(res, "{val:.p$}", p = precision as usize).unwrap();
    } else {
        write!(res, "{val}").unwrap();
    }
    format_text(s, &res, width, dir, b' ');
}

#[must_use]
pub fn fix_from(from: i32, s: &str) -> usize {
    let size = s.len() as i32;
    let mut f = if from < 0 { from + size } else { from };
    if f < 0 {
        return 0;
    }
    let b = s.as_bytes();
    // when from is inside a UTF-8 token: decrease it
    while f > 0 && b[f as usize] >= 128 && b[f as usize] < 192 {
        f -= 1;
    }
    f as usize
}

#[must_use]
pub fn fix_till(till: i32, from: usize, s: &str) -> usize {
    let size = s.len() as i32;
    let mut t = if till == i32::MIN {
        from as i32 + 1
    } else if till < 0 {
        till + size
    } else if till > size {
        size
    } else {
        till
    };
    if t < from as i32 || t > size {
        return from;
    }
    let b = s.as_bytes();
    // when till is inside a UTF-8 token: increase it
    while t < size && b[t as usize] >= 128 && b[t as usize] < 192 {
        t += 1;
    }
    t as usize
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn test_layouts() {
        let mut s = String::new();
        format_text(&mut s, "aa", 5, 0, b'_');
        assert_eq!("_aa__", s);
        s.clear();
        format_text(&mut s, "aa", 6, 0, b'_');
        assert_eq!("__aa__", s);
        s.clear();
        format_int(&mut s, 0x1234, 16, 0, b' ', false, true);
        assert_eq!("0x1234", s);
        s.clear();
        format_long(&mut s, 0x123_4567, 16, 0, b' ', false, true, 1);
        assert_eq!("0x1234567", s);
        s.clear();
        format_int(&mut s, -1, 10, 3, b'0', false, false);
        assert_eq!("-01", s);
        s.clear();
        format_int(&mut s, -1, 10, 4, b'0', false, false);
        assert_eq!("-001", s);
        s.clear();
        format_long(&mut s, -1, 10, 3, b'0', false, false, 1);
        assert_eq!("-01", s);
        s.clear();
        format_int(&mut s, 1, 10, 3, b'0', true, false);
        assert_eq!("+01", s);
    }

    // --- T1-31: checked integer arithmetic tests ---

    #[test]
    fn add_int_normal() {
        assert_eq!(op_add_int(3, 4), 7);
    }

    #[test]
    fn add_int_null_propagation() {
        assert_eq!(op_add_int(i32::MIN, 5), i32::MIN);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "integer overflow")]
    fn add_int_overflow() {
        let _ = op_add_int(i32::MAX, 1);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "integer overflow")]
    fn sub_int_overflow() {
        let _ = op_min_int(i32::MIN + 1, 2);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "integer overflow")]
    fn mul_int_overflow() {
        let _ = op_mul_int(i32::MAX, 2);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "integer null-sentinel collision")]
    fn sub_int_sentinel() {
        let _ = op_min_int(-2_147_483_647, 1);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "integer null-sentinel collision")]
    fn and_int_sentinel() {
        // 0x80000001 & 0x80000002 = 0x80000000 = i32::MIN
        let _ = op_logical_and_int(i32::MIN + 1, i32::MIN + 2);
    }

    #[test]
    fn add_long_normal() {
        assert_eq!(op_add_long(100, 200), 300);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "long overflow")]
    fn add_long_overflow() {
        let _ = op_add_long(i64::MAX, 1);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "long null-sentinel collision")]
    fn sub_long_sentinel() {
        let _ = op_min_long(i64::MIN + 1, 1);
    }

    /// compile-time guard — `_int` functions must never reference `i64::MIN`.
    /// Integer paths use `i32::MIN` as their null sentinel.  Any `i64::MIN` check
    /// in an `_int` function would add an unnecessary branch on every integer op.
    #[test]
    fn no_i64_sentinel_in_int_functions() {
        let src = include_str!("ops.rs");
        // Split the source into function bodies by scanning for `fn op_` / `fn format_`
        // boundaries.  For every function whose name ends with `_int(`, assert that
        // its body does not contain `i64::MIN`.
        let mut in_int_fn = false;
        let mut fn_name = String::new();
        for line in src.lines() {
            if let Some(rest) = line.strip_prefix("pub fn ") {
                // Conversion functions (op_conv_*_from_int) correctly map i32::MIN → i64::MIN
                // or i32::MIN → f64::NAN.  Exclude them from the audit.
                in_int_fn = rest.contains("_int(") && !rest.contains("_from_int(");
                fn_name = rest.split('(').next().unwrap_or("").to_string();
            }
            if in_int_fn {
                assert!(
                    !line.contains("i64::MIN"),
                    "O3 violation: `_int` function `{fn_name}` references i64::MIN — \
                     integer paths must use i32::MIN, not the long sentinel"
                );
            }
        }
    }
}
