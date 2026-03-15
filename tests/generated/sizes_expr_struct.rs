#![allow(unused_imports)]
#![allow(unused_parens)]
#![allow(unused_variables)]
#![allow(unreachable_code)]
#![allow(unused_mut)]
#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(redundant_semicolons)]
#![allow(unused_assignments)]
#![allow(clippy::double_parens)]
#![allow(clippy::unused_unit)]

extern crate loft;
use loft::database::Stores;
use loft::keys::{DbRef, Str, Key, Content};
use loft::ops;
use loft::vector;

fn init(db: &mut Stores) {
    let e = db.enumerate("En");
    db.value(e, "V1", u16::MAX);
    db.value(e, "V2", u16::MAX);
    let s = db.structure("S", 0); // 19
    db.field(s, "a", 0);
    db.field(s, "b", 1);
    db.field(s, "c", 18);
    db.finish();
}

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___work_1: String = "".to_string();
  let mut var_test_value: i32 = { //block_2: integer
    13_i32
    } /*block_2: integer*/;
  let _pre13 = { //Formatted string_3: text["__work_1"]
    var___work_1 = "Test failed ".to_string();
    ops::format_long(&mut var___work_1, external::op_conv_long_from_int((var_test_value)), 10 as u8, 0_i32, 32 as u8, false, false);
    var___work_1 += " != 13";
    &var___work_1
    } /*Formatted string_3: text["__work_1"]*/;
  n_assert(stores, (var_test_value) == (13_i32), _pre13, "expr_struct", 7_i32);
  ;
  } /*block_1: void*/

#[test]
fn code_expr_struct() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
