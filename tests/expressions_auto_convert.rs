// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

#![allow(unused_imports)]
#![allow(unused_parens)]
#![allow(unused_variables)]

extern crate loft;
use loft::database::Stores;
use loft::ops::*;

fn init(db: &mut Stores) {}

fn test(stores: &mut Stores) -> i64 {
    op_mul_long((10_i64), (op_conv_long_from_int(2_i32)))
}

#[test]
fn code_auto_convert() {
    let mut db = Stores::new();
    init(&mut db);
    assert_eq!(20, test(&mut db));
}
