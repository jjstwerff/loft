// Copyright (c) 2023-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_wrap)]
// Should be removed when actually in use
#![allow(dead_code)]

use crate::keys;
use crate::keys::{Content, DbRef, Key};
use crate::store::Store;
use std::cmp::Ordering;

// Negative values in LEFT or RIGHT position are back links to higher data.
// For normal traversal they should be treated as a leaf node (value 0).
static RB_LEFT: u32 = 0;
static RB_RIGHT: u32 = 4;
static RB_FLAG: u32 = 8;
static RB_MAX_DEPTH: u32 = 30;

// Normally rec holds the position towards the LEFT, RiGHT, FLAG fields.
// However, the compare functions assume pos = 0 for records outside a vector.
/**
Get the lowest matching record, with `before` return the record before the lowest.
The `fields` parameter points to the position inside the record of the fields.
*/
#[must_use]
pub fn find(
    data: &DbRef,
    before: bool,
    fields: u16,
    stores: &[Store],
    keys: &[Key],
    key: &[Content],
) -> u32 {
    let store = keys::store(data, stores);
    let mut rec = store.get_int(data.rec, data.pos) as u32;
    let mut result = DbRef {
        store_nr: data.store_nr,
        rec: 0,
        pos: 0,
    };
    let mut cmp = Ordering::Equal;
    while rec > 0 {
        result.rec = rec;
        result.pos = 8;
        cmp = keys::key_compare(key, &result, stores, keys);
        let action = if cmp == Ordering::Equal {
            if before {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        } else {
            cmp
        };
        let to = store.get_int(
            rec,
            u32::from(fields)
                + if action == Ordering::Less {
                    RB_LEFT
                } else {
                    RB_RIGHT
                },
        );
        rec = if to >= 0 { to as u32 } else { 0 };
    }
    if cmp == Ordering::Equal {
        result.pos = u32::from(fields);
        if before {
            return previous(store, &result);
        }
        return next(store, &result);
    }
    result.rec
}

/// Add a new record
/// Always the same: `store`, `rec_pos`, `top_pos`, `lower`
pub fn add(data: &DbRef, record: &DbRef, fields: u16, stores: &mut [Store], keys: &[Key]) {
    let store = keys::mut_store(data, stores);
    let mut rec = *record;
    rec.pos = u32::from(fields);
    store.set_byte(rec.rec, rec.pos + RB_FLAG, 0, 0);
    store.set_int(rec.rec, rec.pos + RB_LEFT, 0);
    store.set_int(rec.rec, rec.pos + RB_RIGHT, 0);
    let new_top = if store.get_int(data.rec, data.pos) == 0 {
        rec.rec
    } else {
        let top = store.get_int(data.rec, data.pos);
        put(0, top, &rec, 0, 0, keys, stores) as u32
    };
    if new_top == 0 || new_top == u32::MAX {
        return; // problem encountered: probably duplicate key
    }
    let store = keys::mut_store(data, stores);
    store.set_int(data.rec, data.pos, new_top as i32);
    store.set_byte(new_top, rec.pos + RB_FLAG, 0, 0);
}

#[must_use]
/// Return the first element in the tree
pub fn first(data: &DbRef, fields: u16, stores: &[Store]) -> DbRef {
    end(data, fields, stores, RB_LEFT)
}

#[must_use]
/// Return the last element in the tree
pub fn last(data: &DbRef, fields: u16, stores: &[Store]) -> DbRef {
    end(data, fields, stores, RB_RIGHT)
}

fn end(data: &DbRef, fields: u16, stores: &[Store], forward: u32) -> DbRef {
    let store = keys::store(data, stores);
    let mut i = store.get_int(data.rec, data.pos);
    while i > 0 && store.get_int(i as u32, u32::from(fields) + forward) > 0 {
        i = store.get_int(i as u32, u32::from(fields) + forward);
    }
    DbRef {
        store_nr: data.store_nr,
        rec: i as u32,
        pos: 8,
    }
}

fn left(store: &Store, r: &DbRef) -> i32 {
    store.get_int(r.rec, r.pos + RB_LEFT)
}

fn right(store: &Store, r: &DbRef) -> i32 {
    store.get_int(r.rec, r.pos + RB_RIGHT)
}

fn flag(store: &Store, r: &DbRef) -> bool {
    store.get_byte(r.rec, r.pos + RB_FLAG, 0) == 1
}

fn set_left(store: &mut Store, r: &DbRef, to: i32) {
    store.set_int(r.rec, r.pos + RB_LEFT, to);
}

fn set_right(store: &mut Store, r: &DbRef, to: i32) {
    store.set_int(r.rec, r.pos + RB_RIGHT, to);
}

fn set_flag(store: &mut Store, r: &DbRef, to: bool) {
    store.set_byte(r.rec, r.pos + RB_FLAG, 0, i32::from(to));
}

/// Find the correct position to insert the element
fn put(
    depth: u32,
    pos: i32,
    rec: &DbRef,
    l: i32,
    r: i32,
    keys: &[Key],
    stores: &mut [Store],
) -> i32 {
    if depth > RB_MAX_DEPTH {
        return 0;
    }
    if pos <= 0 {
        let store = &mut stores[rec.store_nr as usize];
        set_flag(store, rec, true);
        set_left(store, rec, -l);
        set_right(store, rec, -r);
        return rec.rec as i32;
    }
    if pos as u32 == rec.rec {
        // duplicate record
        return rec.rec as i32;
    }
    let current = to_ref(rec, pos);
    let cmp = keys::compare(&compare(rec), &compare(&current), stores, keys);
    if cmp == Ordering::Less {
        let next = left(keys::store(rec, stores), &current);
        let p = put(depth + 1, next, rec, l, pos, keys, stores);
        if p < 0 {
            return -1;
        }
        set_left(keys::mut_store(rec, stores), &current, p);
    } else if cmp == Ordering::Greater {
        let next = right(keys::store(rec, stores), &current);
        let p = put(depth + 1, next, rec, pos, r, keys, stores);
        if p < 0 {
            return -1;
        }
        set_right(keys::mut_store(rec, stores), &current, p);
    } else {
        // double keys
        return -1;
    }
    balance(keys::mut_store(rec, stores), &current)
}

fn to_ref(rec: &DbRef, to: i32) -> DbRef {
    assert!(to >= 0, "incorrect to_ref");
    DbRef {
        store_nr: rec.store_nr,
        rec: to as u32,
        pos: rec.pos,
    }
}

fn compare(rec: &DbRef) -> DbRef {
    DbRef {
        store_nr: rec.store_nr,
        rec: rec.rec,
        pos: 8,
    }
}

/// When the position is found, re-balance the tree after inserting
fn balance(store: &mut Store, rec: &DbRef) -> i32 {
    if flag(store, rec) {
        return rec.rec as i32;
    }
    let l = left(store, rec);
    let r = right(store, rec);
    if l > 0 && flag(store, &to_ref(rec, l)) {
        let ll = left(store, &to_ref(rec, l));
        if ll > 0 && flag(store, &to_ref(rec, ll)) {
            return fix_ll(store, rec, l, ll);
        }
        let lr = right(store, &to_ref(rec, l));
        if lr > 0 && flag(store, &to_ref(rec, lr)) {
            return fix_lr(store, rec, l, lr);
        }
    }
    if r > 0 && flag(store, &to_ref(rec, r)) {
        let rl = left(store, &to_ref(rec, r));
        if rl > 0 && flag(store, &to_ref(rec, rl)) {
            return fix_rl(store, rec, r, rl);
        }
        let rr = right(store, &to_ref(rec, r));
        if rr > 0 && flag(store, &to_ref(rec, rr)) {
            return fix_rr(store, rec, r, rr);
        }
    }
    rec.rec as i32
}

/// Black, rec, Node (Red, l, Node (Red, ll, a, b), c), d
/// -> Red, l, Node (Black, ll, a, b), Node (Black, rec, c, d)
fn fix_ll(store: &mut Store, rec: &DbRef, l: i32, ll: i32) -> i32 {
    let c = right(store, &to_ref(rec, l));
    set_right(store, &to_ref(rec, l), rec.rec as i32);
    set_flag(store, &to_ref(rec, ll), false);
    set_left(store, rec, if c < 0 { -l } else { c });
    l
}

/// Black, rec, Node (Red, l, a, Node (Red, lr, b, c)), d
/// -> Red, lr, Node (Black, l, a, b), Node (Black, rec, c, d)
fn fix_lr(store: &mut Store, rec: &DbRef, l: i32, lr: i32) -> i32 {
    let b = left(store, &to_ref(rec, lr));
    let c = right(store, &to_ref(rec, lr));
    set_left(store, &to_ref(rec, lr), l);
    set_right(store, &to_ref(rec, lr), rec.rec as i32);
    set_flag(store, &to_ref(rec, l), false);
    set_right(store, &to_ref(rec, l), if b < 0 { -lr } else { b });
    set_left(store, rec, if c < 0 { -lr } else { c });
    lr
}

/// Black, p, a, Node (Red, r, Node (Red, rl, b, c), d)
/// -> Red, rl, Node (Black, p, a, b), Node (Black, r, c, d)
fn fix_rl(store: &mut Store, rec: &DbRef, r: i32, rl: i32) -> i32 {
    let b = left(store, &to_ref(rec, rl));
    let c = right(store, &to_ref(rec, rl));
    set_left(store, &to_ref(rec, rl), rec.rec as i32);
    set_right(store, &to_ref(rec, rl), r);
    set_right(store, rec, if b < 0 { -rl } else { b });
    set_flag(store, &to_ref(rec, r), false);
    set_left(store, &to_ref(rec, r), if c < 0 { -rl } else { c });
    rl
}

/// Black, p, a, Node (Red, r, b, Node (Red, rr, c, d))
/// -> Red, r, Node (Black, p, a, b), Node (Black, rr, c, d)
fn fix_rr(store: &mut Store, rec: &DbRef, r: i32, rr: i32) -> i32 {
    let b = left(store, &to_ref(rec, r));
    set_left(store, &to_ref(rec, r), rec.rec as i32);
    set_right(store, rec, if b < 0 { -r } else { b });
    set_flag(store, &to_ref(rec, rr), false);
    r
}

/// Walk to the element to remove
fn remove_iter(
    rec: &DbRef,
    depth: u32,
    pos: u32,
    black: &mut bool,
    stores: &mut [Store],
    keys: &[Key],
) -> u32 {
    assert!(depth <= RB_MAX_DEPTH, "Too many iterations");
    assert_ne!(pos, 0, "Item not found");
    let mut pos_ref = to_ref(rec, pos as i32);
    let compare_to;
    if pos == rec.rec {
        pos_ref.rec = remove_elm(&pos_ref, depth, black, stores, keys);
        if pos_ref.rec == 0 {
            return 0;
        }
        let store = keys::mut_store(rec, stores);
        if left(store, &pos_ref) <= 0 && right(store, &pos_ref) <= 0 {
            assert!(*black, "Cannot change node to black twice in remove()");
            assert!(
                flag(store, &pos_ref),
                "Child of single-child node should be red"
            );
            set_flag(store, &pos_ref, false);
            *black = false;
            return pos_ref.rec;
        }
        compare_to = -1;
    } else {
        let cmp = keys::compare(&compare(rec), &compare(&pos_ref), stores, keys);
        if cmp == Ordering::Less {
            compare_to = -1;
            let cl = left(keys::store(rec, stores), &pos_ref);
            assert!(cl >= 0, "should be a normal node");
            let mut l = remove_iter(rec, depth + 1, cl as u32, black, stores, keys) as i32;
            let store = keys::mut_store(rec, stores);
            if l == 0 {
                l = left(store, rec);
            }
            set_left(store, &pos_ref, l);
        } else {
            compare_to = 1;
            let cr = right(keys::store(rec, stores), &pos_ref);
            assert!(cr >= 0, "should be a normal node");
            let mut r = remove_iter(rec, depth + 1, cr as u32, black, stores, keys) as i32;
            let store = keys::mut_store(rec, stores);
            if r == 0 {
                r = right(store, rec);
            }
            set_right(store, &pos_ref, r);
        }
    }
    if *black {
        pos_ref.rec = repair(&pos_ref, black, compare_to, stores, keys);
    }
    pos_ref.rec
}

fn remove_elm(
    rec: &DbRef,
    depth: u32,
    black: &mut bool,
    stores: &mut [Store],
    keys: &[Key],
) -> u32 {
    let store = keys::mut_store(rec, stores);
    let l = left(store, rec);
    let r = right(store, rec);
    let rd = flag(store, rec);
    if l <= 0 {
        // left is empty: return right as replacement
        *black = !rd;
        if r <= 0 {
            return 0;
        }
        assert!(!rd, "Expected node with single-child to be black");
        if left(store, &to_ref(rec, r)) < 0 {
            set_left(store, &to_ref(rec, r), l);
        }
        return r as u32;
    }
    if r <= 0 {
        // left is empty: return left as replacement
        *black = !rd;
        assert!(!rd, "Expected node with single-child to be black");
        if right(store, &to_ref(rec, l)) < 0 {
            set_right(store, &to_ref(rec, l), r);
        }
        return l as u32;
    }
    // both left and right as not empty: remove previous element in tree
    // (=max(l)) and then make that the replacement
    let pos = max(store, &to_ref(rec, l));
    let mut new_left = remove_iter(&pos, depth + 1, l as u32, black, stores, keys) as i32;
    let store = keys::mut_store(rec, stores);
    if new_left == 0 {
        new_left = left(store, &pos);
    }
    set_right(store, &pos, r);
    set_flag(store, &pos, rd);
    set_left(store, &pos, new_left);
    let pv = previous(store, &pos);
    let nx = next(store, &pos);
    if pv > 0 && right(store, &to_ref(rec, pv as i32)) < 0 {
        set_right(store, &to_ref(rec, pv as i32), -(pos.rec as i32));
    }
    if nx > 0 && left(store, &to_ref(rec, nx as i32)) < 0 {
        set_left(store, &to_ref(rec, nx as i32), -(pos.rec as i32));
    }
    pos.rec
}

fn repair(
    rec: &DbRef,
    black: &mut bool,
    compare_to: i32,
    stores: &mut [Store],
    keys: &[Key],
) -> u32 {
    let mut repair1 = 0;
    let mut repair2 = 0;
    let store = keys::mut_store(rec, stores);
    match compare_to.cmp(&0) {
        Ordering::Less => {
            let r = right(store, rec);
            if r < 0 {
                // Expecting a normal node
                return 0;
            }
            child_to_red(store, &to_ref(rec, r), &mut repair1, &mut repair2);
        }
        Ordering::Greater => {
            let l = left(store, rec);
            if l < 0 {
                // Expecting a normal node
                return 0;
            }
            child_to_red(store, &to_ref(rec, l), &mut repair1, &mut repair2);
        }
        Ordering::Equal => {}
    }
    if flag(store, rec) {
        set_flag(store, rec, false);
        *black = false;
    }
    let mut p = *rec;
    if repair1 != 0 {
        p = do_repair(&to_ref(rec, repair1), &p, 0, stores, keys);
    }
    if repair2 != 0 {
        p = do_repair(&to_ref(rec, repair2), &p, 0, stores, keys);
    }
    if *black && flag(keys::store(rec, stores), &p) {
        set_flag(keys::mut_store(rec, stores), &p, false);
        *black = false;
    }
    p.rec
}

fn child_to_red(store: &mut Store, rec: &DbRef, l: &mut i32, r: &mut i32) {
    if flag(store, rec) {
        *l = left(store, rec);
        assert!(*l > 0, "Incorrect child_to_red");
        set_flag(store, &to_ref(rec, *l), true);
        *r = right(store, rec);
        assert!(*r > 0, "Incorrect child_to_red");
        set_flag(store, &to_ref(rec, *r), true);
    } else {
        set_flag(store, rec, true);
        *l = rec.rec as i32;
    }
}

/// Find rec starting at pos.
/// Balance pos for each level down.
fn do_repair(rec: &DbRef, pos: &DbRef, depth: u32, stores: &mut [Store], keys: &[Key]) -> DbRef {
    assert!(depth <= RB_MAX_DEPTH, "Too many iterations");
    let store = keys::mut_store(rec, stores);
    let bal = balance(store, pos);
    assert!(bal > 0, "Incorrect balance");
    let result = to_ref(rec, bal);
    if result.rec != rec.rec {
        let cmp = keys::compare(&compare(rec), &compare(&result), stores, keys);
        if cmp == Ordering::Less {
            let l = left(keys::store(rec, stores), &result);
            assert!(l > 0, "Incorrect repair left");
            let r = do_repair(rec, &to_ref(rec, l), depth + 1, stores, keys);
            set_left(keys::mut_store(rec, stores), &result, r.rec as i32);
        } else {
            let r = right(keys::store(rec, stores), &result);
            assert!(r > 0, "Incorrect repair right");
            let r = do_repair(rec, &to_ref(rec, r), depth + 1, stores, keys);
            set_right(keys::mut_store(rec, stores), &result, r.rec as i32);
        }
    }
    result
}

/// Safe validation version of `rb_first`
fn min(store: &Store, rec: &DbRef) -> DbRef {
    let mut depth = 0;
    if rec.rec == 0 {
        return *rec;
    }
    let mut p = *rec;
    loop {
        let l = left(store, &p);
        if l <= 0 || depth > RB_MAX_DEPTH {
            return p;
        }
        p.rec = l as u32;
        depth += 1;
    }
}

/// Safe validation version of `rb_last`
fn max(store: &Store, rec: &DbRef) -> DbRef {
    let mut depth = 0;
    if rec.rec == 0 {
        return *rec;
    }
    let mut p = *rec;
    loop {
        let l = right(store, &p);
        if l <= 0 || depth > RB_MAX_DEPTH {
            return p;
        }
        p.rec = l as u32;
        depth += 1;
    }
}

/// Walk through the tree validating ordering, returning the number of elements.
/// Validating that each branch has the same number of black elements.
/// Check if no two elements into the tree are both red.
fn verify(rec: &DbRef, blacks: u32, max_blacks: &mut u32, stores: &[Store], keys: &[Key]) -> u32 {
    assert!(blacks <= RB_MAX_DEPTH, "Too deep structure");
    let store = keys::store(rec, stores);
    let l = left(store, rec);
    let r = right(store, rec);
    assert!(
        l.unsigned_abs() != rec.rec && r.unsigned_abs() != rec.rec,
        "Linked to self on {rec:?}"
    );
    let nb = blacks + u32::from(!flag(store, rec));
    //println!("rec:{} l:{l}, r:{r} nb:{nb} max:{max_blacks}", rec.rec);
    1 + v_side(rec, l, true, nb, max_blacks, stores, keys)
        + v_side(rec, r, false, nb, max_blacks, stores, keys)
}

fn v_side(
    rec: &DbRef,
    side: i32,
    left: bool,
    depth: u32,
    max_blacks: &mut u32,
    stores: &[Store],
    keys: &[Key],
) -> u32 {
    assert!(depth < RB_MAX_DEPTH, "Too deep structure");
    if side > 0 {
        let s = to_ref(rec, side);
        // println!("side:{side}={:?} rec:{}={:?} depth:{depth}", keys::get_key(&compare(&s), stores, keys),rec.rec,keys::get_key(&compare(rec), stores, keys));
        let cmp = keys::compare(&compare(&s), &compare(rec), stores, keys);
        assert!(
            rec.rec != s.rec && cmp != Ordering::Equal,
            "Duplicate key {rec:?} and {s:?}"
        );
        assert_eq!(
            u8::from(left) ^ u8::from(cmp == Ordering::Less),
            0,
            "Ordering not correct"
        );
        let store = keys::store(rec, stores);
        assert!(
            !flag(store, rec) || !flag(store, &s),
            "Two adjacent red nodes {rec:?} and {s:?}"
        );
        verify(&s, depth, max_blacks, stores, keys)
    } else if *max_blacks == 0 {
        *max_blacks = depth;
        0
    } else if *max_blacks != depth {
        panic!("Not balanced {depth} != {} on {}", *max_blacks, rec.rec);
    } else {
        0
    }
}

fn verify_walk(start: &DbRef, end: &DbRef, dir: bool, size: u32, stores: &[Store], keys: &[Key]) {
    let mut step = 1;
    let mut elm = *start;
    let store = keys::store(start, stores);
    while elm.rec != end.rec {
        assert!(step <= size, "Too long at {elm:?}");
        let n = if dir {
            next(store, &elm)
        } else {
            previous(store, &elm)
        };
        assert_ne!(n, 0, "Incorrect at {elm:?}");
        let mut cmp = keys::compare(
            &compare(&to_ref(start, n as i32)),
            &compare(&elm),
            stores,
            keys,
        );
        if dir {
            cmp = cmp.reverse();
        }
        assert_eq!(cmp, Ordering::Less, "Not ascending at {elm:?}");
        elm.rec = n;
        step += 1;
    }
    assert_eq!(step, size, "Incorrect length");
}

fn moving(store: &Store, rec: &DbRef, first: u32, second: u32) -> u32 {
    if rec.rec == 0 {
        return 0;
    }
    let mut r = store.get_int(rec.rec, rec.pos + first);
    let mut depth = 0;
    while r > 0 && store.get_int(r as u32, rec.pos + second) > 0 {
        r = store.get_int(r as u32, rec.pos + second);
        if depth > RB_MAX_DEPTH {
            return 0;
        }
        depth += 1;
    }
    r.unsigned_abs()
}

/// Step to the next element in the tree
#[must_use]
pub fn next(store: &Store, rec: &DbRef) -> u32 {
    moving(store, rec, RB_RIGHT, RB_LEFT)
}

/// Step to the previous element in the tree
#[must_use]
pub fn previous(store: &Store, rec: &DbRef) -> u32 {
    moving(store, rec, RB_LEFT, RB_RIGHT)
}

/// Validate the tree
/// # Panics
/// When the tree is not correctly defined
pub fn validate(data: &DbRef, fields: u16, stores: &[Store], keys: &[Key]) {
    let store = keys::store(data, stores);
    let rec = DbRef {
        store_nr: data.store_nr,
        rec: store.get_int(data.rec, data.pos) as u32,
        pos: u32::from(fields),
    };
    if rec.rec == 0 {
        return;
    }
    assert!(rec.rec != 0 && !flag(store, &rec), "Root is not black");
    let mut max_blacks = 0;
    let v = verify(&rec, 0, &mut max_blacks, stores, keys);
    let min = min(store, &rec);
    assert_eq!(
        previous(store, &min),
        0,
        "Incorrect min element {}",
        min.rec
    );
    let max = max(store, &rec);
    assert_eq!(next(store, &max), 0, "Incorrect max element {}", max.rec);
    verify_walk(&min, &max, true, v, stores, keys);
    verify_walk(&max, &min, false, v, stores, keys);
}

pub fn remove(data: &DbRef, rec: &DbRef, fields: u16, stores: &mut [Store], keys: &[Key]) {
    let mut black = false;
    let top = keys::store(data, stores).get_int(data.rec, data.pos) as u32;
    let r = DbRef {
        store_nr: rec.store_nr,
        rec: rec.rec,
        pos: u32::from(fields),
    };
    let new_top = remove_iter(&r, 0, top, &mut black, stores, keys);
    let s = keys::mut_store(data, stores);
    // Always update the root pointer — when new_top == 0 the tree is now empty
    // and the old root must be cleared; skipping this update caused T0-3 (off-by-one
    // "phantom record" after removing all elements).
    s.set_int(data.rec, data.pos, new_top as i32);
    if new_top > 0 {
        s.set_byte(new_top, u32::from(fields) + RB_FLAG, 0, 0);
    }
}
